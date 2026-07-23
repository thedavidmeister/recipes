//! The equipment reading (#81): what a recipe needs you to own, read off the service.
//!
//! A peer of the ingredient reading (#11) and the step reading (#74/#75/#76), and the
//! same pipeline: the app offers recipes that have not been read, a worker pulls them,
//! a model reads them, and the app validates and writes every reading. No model code,
//! prompt or provider key lives here (#59).
//!
//! What is different is what the reading is *for*. A kitchen selects its equipment
//! from this vocabulary and may not invent items (#81), because the only purpose of
//! knowing what a kitchen owns is matching it against recipes. That makes the set of
//! distinct names a first-class product of this table — see [`vocabulary`] — and makes
//! normalisation a validity rule rather than a tidy-up.

use std::collections::HashMap;

use libsql::Connection;
use recipe_core::equipment::{self, RequiredEquipment};
use serde::{Deserialize, Serialize};

use crate::{derive, runs};

type RecipeKey = (String, String);

/// A recipe waiting to be read: enough to know what it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingEquipmentRecipe {
    pub source: String,
    pub id: String,
    pub instructions: String,
}

/// Recipes with a method but no equipment reading yet, capped at `limit`.
///
/// Recipes with a blank method are excluded: there is nothing to read equipment from,
/// and offering one would keep it in the queue forever — the worker reads until
/// `pending` is empty, so a recipe that can never earn a reading never leaves.
pub async fn pending(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<PendingEquipmentRecipe>> {
    let limit = limit.max(1) as i64;
    let mut rows = conn
        .query(
            "SELECT r.source, r.id, r.instructions
             FROM recipes r
             LEFT JOIN equipment_structures e ON e.source = r.source AND e.id = r.id
             WHERE e.id IS NULL
               AND r.instructions IS NOT NULL
               AND trim(r.instructions) <> ''
             LIMIT ?1",
            libsql::params![limit],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(PendingEquipmentRecipe {
            source: row.get::<String>(0)?,
            id: row.get::<String>(1)?,
            instructions: row.get::<String>(2)?,
        });
    }
    Ok(out)
}

/// One recipe's equipment reading as the worker submits it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubmittedEquipment {
    pub source: String,
    pub id: String,
    pub equipment: Vec<RequiredEquipment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejection {
    pub source: String,
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct SubmitReport {
    pub accepted: usize,
    pub derived: usize,
    pub rejected: Vec<Rejection>,
}

/// Store a batch of readings, then re-derive the recipes that accepted one.
///
/// The derive run is allocated **after** storage, so a concurrent ingest that derived
/// `recipes` first cannot leave an accepted reading unattached.
///
/// An **empty reading is rejected**, like an empty step reading. Practically every
/// recipe requires something — a salad still needs a bowl, a knife and a board — so an
/// empty list means the model read for appliances and ignored preparation. Storing it
/// would be worse than useless: the recipe would leave the queue permanently, and a
/// kitchen owning no knife would appear able to cook it.
pub async fn submit(
    conn: &Connection,
    items: Vec<SubmittedEquipment>,
    model: &str,
) -> anyhow::Result<SubmitReport> {
    let mut report = SubmitReport::default();
    let mut accepted: Vec<RecipeKey> = Vec::new();

    let store_run = runs::begin(conn, "enrich_equipment").await?;
    for item in items {
        if !recipe_exists(conn, &item.source, &item.id).await? {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "no such recipe".into(),
            });
            continue;
        }
        if item.equipment.is_empty() {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "empty equipment reading — every recipe needs something, a salad still needs a bowl and a knife"
                    .into(),
            });
            continue;
        }
        if let Err(reason) = equipment::validate(&item.equipment) {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!("invalid equipment reading: {reason}"),
            });
            continue;
        }
        let wrote = store(
            conn,
            &item.source,
            &item.id,
            &item.equipment,
            model,
            store_run,
        )
        .await?;
        if wrote {
            accepted.push((item.source, item.id));
            report.accepted += 1;
        } else {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "superseded — a newer run already stored a reading".into(),
            });
        }
    }
    runs::finish(conn, store_run, runs::COMPLETED).await?;

    if !accepted.is_empty() {
        let derive_run = runs::begin(conn, "derive").await?;
        report.derived = derive::derive_recipes(conn, &accepted, derive_run)
            .await?
            .derived;
        runs::finish(conn, derive_run, runs::COMPLETED).await?;
    }
    Ok(report)
}

async fn recipe_exists(conn: &Connection, source: &str, id: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM recipes WHERE source = ?1 AND id = ?2",
            libsql::params![source.to_owned(), id.to_owned()],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// Every distinct piece of equipment the corpus knows about, alphabetically.
///
/// This is the list a kitchen picks from, and the reason it exists at all: equipment a
/// kitchen owns is only useful if a recipe can ask for it by the same name (#81). A
/// name that appears in no recipe is not offered, because owning it could never change
/// what you are able to cook.
pub async fn vocabulary(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let readings = load(conn).await?;
    let mut names: Vec<String> = readings
        .into_values()
        .flatten()
        .map(|e| e.item)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    names.sort();
    Ok(names)
}

/// The vocabulary's answer for one name: the normalised form if the corpus asks for
/// it, `None` if it does not.
///
/// Normalising the caller's input first means a kitchen may type "Frying Pan" and be
/// understood — the strictness is about *which* items exist, not about punishing
/// someone for a capital letter. What it will not do is invent an item.
pub async fn normalise_known(conn: &Connection, raw: &str) -> anyhow::Result<Option<String>> {
    let wanted = equipment::normalise(raw);
    if wanted.is_empty() {
        return Ok(None);
    }
    Ok(vocabulary(conn).await?.into_iter().find(|k| *k == wanted))
}

/// Load every reading so [`crate::derive`] can reattach in memory — one query, not a
/// lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Vec<RequiredEquipment>>> {
    let mut rows = conn
        .query(
            "SELECT source, id, structured FROM equipment_structures",
            (),
        )
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let structured: String = row.get(2)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal.
        if let Ok(items) = serde_json::from_str::<Vec<RequiredEquipment>>(&structured) {
            map.insert((source, id), items);
        }
    }
    Ok(map)
}

/// Reattach a recipe's reading onto `recipe.equipment` in place. A recipe with no row
/// keeps `[]` — which reads as "not known yet", the degrade-not-die state.
pub fn attach(
    by_recipe: &HashMap<RecipeKey, Vec<RequiredEquipment>>,
    source: &str,
    id: &str,
    equipment: &mut Vec<RequiredEquipment>,
) {
    if let Some(read) = by_recipe.get(&(source.to_owned(), id.to_owned())) {
        *equipment = read.clone();
    }
}

/// Write one reading, guarded on `run_id` so a stale run cannot clobber a newer one.
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    items: &[RequiredEquipment],
    model: &str,
    run_id: i64,
) -> anyhow::Result<bool> {
    let structured = serde_json::to_string(items)?;
    let affected = conn
        .execute(
            "INSERT INTO equipment_structures (source, id, structured, model, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source, id) DO UPDATE SET
                structured = excluded.structured,
                model      = excluded.model,
                created_at = unixepoch(),
                run_id     = excluded.run_id
             WHERE excluded.run_id >= equipment_structures.run_id",
            libsql::params![source, id, structured, model, run_id],
        )
        .await?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    async fn insert_recipe(conn: &Connection, id: &str, instructions: &str) {
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', ?1, 'T', '[]', ?2)",
            libsql::params![id, instructions],
        )
        .await
        .unwrap();
    }

    fn eq(item: &str) -> RequiredEquipment {
        RequiredEquipment { item: item.into() }
    }

    fn submitted(id: &str, items: &[&str]) -> SubmittedEquipment {
        SubmittedEquipment {
            source: "themealdb".into(),
            id: id.into(),
            equipment: items.iter().map(|i| eq(i)).collect(),
        }
    }

    /// The queue offers what has not been read, and stops offering it once it has.
    #[tokio::test]
    async fn pending_empties_as_readings_land() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chop. Fry.").await;

        assert_eq!(pending(&conn, 10).await.unwrap().len(), 1);

        let report = submit(&conn, vec![submitted("1", &["knife", "wok"])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 1);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        assert!(
            pending(&conn, 10).await.unwrap().is_empty(),
            "a read recipe leaves the queue"
        );
    }

    /// A recipe with no method can never earn a reading, so offering it would keep the
    /// worker looping on it forever.
    #[tokio::test]
    async fn a_recipe_without_a_method_is_never_offered() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "   ").await;
        assert!(pending(&conn, 10).await.unwrap().is_empty());
    }

    /// A salad still needs a bowl and a knife. An empty reading means the model read
    /// for appliances and ignored preparation — so it is refused, and the recipe stays
    /// in the queue to be read again rather than leaving it with nothing captured.
    #[tokio::test]
    async fn an_empty_reading_is_refused() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Toss the leaves.").await;

        let report = submit(&conn, vec![submitted("1", &[])], "m").await.unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.rejected.len(), 1);
        assert!(
            report.rejected[0].reason.contains("still needs a bowl"),
            "{}",
            report.rejected[0].reason
        );
        assert_eq!(
            pending(&conn, 10).await.unwrap().len(),
            1,
            "and it is offered again"
        );
    }

    /// Names must arrive normalised, because a kitchen picks from this vocabulary and
    /// "Wok" would be a second, unmatchable entry beside "wok".
    #[tokio::test]
    async fn an_unnormalised_name_is_refused() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Fry.").await;

        let report = submit(&conn, vec![submitted("1", &["Wok"])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("not normalised"));
    }

    /// The vocabulary is the union across the corpus, deduplicated and ordered — the
    /// list a kitchen picks from, and the whole of what it may pick from.
    #[tokio::test]
    async fn the_vocabulary_is_every_name_once() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Fry.").await;
        insert_recipe(&conn, "2", "Chop.").await;
        submit(
            &conn,
            vec![
                submitted("1", &["wok", "knife"]),
                submitted("2", &["knife", "chopping board"]),
            ],
            "m",
        )
        .await
        .unwrap();

        assert_eq!(
            vocabulary(&conn).await.unwrap(),
            vec!["chopping board", "knife", "wok"],
            "each name once, in an order a person can scan"
        );
    }

    /// Nothing read means nothing offered — a kitchen cannot pick equipment the corpus
    /// has never asked for, which is the whole point of #81's ruling.
    #[tokio::test]
    async fn an_unread_corpus_offers_no_vocabulary() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Fry.").await;
        assert!(vocabulary(&conn).await.unwrap().is_empty());
    }

    /// A reading for a recipe we do not have is dropped rather than stored against
    /// nothing.
    #[tokio::test]
    async fn a_reading_for_an_unknown_recipe_is_dropped() {
        let conn = conn().await;
        let report = submit(&conn, vec![submitted("nope", &["wok"])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.rejected[0].reason, "no such recipe");
    }
}
