//! Enrichment: the corpus's structured ingredient readings (#11), and the queue an
//! off-Render worker uses to produce them (#59).
//!
//! A reading turns a recipe's raw ingredient line ("1 (14 oz) can chopped
//! tomatoes") into a [`StructuredMeasure`] deterministic code can scale and
//! convert. Producing one is an LLM job — reading messy text into structure — and
//! that job runs **outside this app**. A worker on another machine pulls the work,
//! a model reads the lines, and the worker pushes the results back. The app holds
//! **no** model code, no prompt, and no provider credential: extraction lives
//! entirely in the `recipes-enrich` plugin's `enrich` skill, which drives the loop.
//! Keeping the model out of the service is the point — it is surface it does not need.
//!
//! This module is the two ends of that queue, plus the storage between them:
//!
//! - [`pending`] — recipes with no reading yet, and their ingredient lines. What
//!   the worker pulls (`recipe-backend enrich pull`).
//! - [`submit`] — a batch of the worker's readings, validated (the recipe still
//!   exists, the reading count matches its *current* ingredient list), stored, and
//!   re-derived so the recipe shows them at once. What the worker pushes
//!   (`recipe-backend enrich push`).
//! - [`store`]/[`load`]/[`attach`] — where a reading lands, and the join
//!   [`crate::derive`] performs to hang readings back onto `recipes`.
//!
//! Both `pull` and `push` are **batch commands over the corpus we already hold**,
//! like `derive` — not request paths. There is no new endpoint anyone can hit; the
//! worker runs the binary against the database directly, on the machine that holds
//! the write token.
//!
//! **A capture, not a derivation.** `recipes` is a deterministic derivation of
//! `raw_imports`; a reading is not — a model is non-deterministic and drifts, so a
//! reading is a point-in-time artifact, a peer of `raw_imports`, carrying its
//! provenance (the model id + a timestamp). `pull` only offers recipes with no
//! reading yet; re-reading the corpus with a better model is a deliberate act, not
//! a silent side effect.
//!
//! **Per recipe, not per line.** One reading array is captured per recipe, aligned
//! to its ingredient order, stored as one row keyed by `(source, id)`. That keeps
//! the raw → enrich → derive chain a clean per-`(source, id)` cascade, and lets
//! this be a dedicated table rather than a generic `(kind, json)` container — a
//! future enrichment (nutrition, allergens) is its own table, not a row here.
//!
//! **Degrade-not-die.** Until the worker has run, recipes carry `structured: None`
//! and the corpus serves raw measures. Enrichment is an addition, never a gate.

use std::collections::HashMap;

use libsql::Connection;
use recipe_core::{Ingredient, StructuredMeasure};
use serde::{Deserialize, Serialize};

use crate::derive;

/// A recipe key: `(source, id)`.
type RecipeKey = (String, String);

// --- The pull side: what still needs reading. ----------------------------------

/// One recipe awaiting enrichment: its key and the raw ingredient lines to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingRecipe {
    pub source: String,
    pub id: String,
    pub ingredients: Vec<PendingLine>,
}

/// A single line to read — the raw text as the source wrote it, nothing structured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingLine {
    pub name: String,
    pub measure: Option<String>,
}

/// Recipes with no stored reading yet, capped at `limit`.
///
/// A left join for "no row in `ingredient_structures`". Recipes with no ingredients
/// are excluded in SQL (`json_array_length > 0`): there is nothing to read, and if
/// one were returned the worker's "loop until pending is empty" would never
/// terminate — an empty recipe never earns a reading, so it never leaves the queue.
pub async fn pending(conn: &Connection, limit: usize) -> anyhow::Result<Vec<PendingRecipe>> {
    let limit = limit.max(1) as i64;
    let mut rows = conn
        .query(
            "SELECT r.source, r.id, r.ingredients
             FROM recipes r
             LEFT JOIN ingredient_structures s ON s.source = r.source AND s.id = r.id
             WHERE s.id IS NULL
               AND json_valid(r.ingredients)
               AND json_array_length(r.ingredients) > 0
             LIMIT ?1",
            libsql::params![limit],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let json: String = row.get(2)?;
        // Parsed for the line text. `json_array_length` already required a JSON
        // array, so a parse failure here is not expected — skip rather than fail the
        // whole pull if it somehow happens.
        let ingredients: Vec<Ingredient> = serde_json::from_str(&json).unwrap_or_default();
        if ingredients.is_empty() {
            continue;
        }
        out.push(PendingRecipe {
            source,
            id,
            ingredients: ingredients
                .into_iter()
                .map(|i| PendingLine {
                    name: i.name,
                    measure: i.measure,
                })
                .collect(),
        });
    }
    Ok(out)
}

// --- The push side: the worker's readings. -------------------------------------

/// One recipe's readings as the worker submits them: the key, and the readings in
/// the recipe's ingredient order. Provenance (the model) is stamped by the `push`
/// command from its environment, not carried here — the skill does not hardcode a
/// model id.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubmittedReadings {
    pub source: String,
    pub id: String,
    pub readings: Vec<StructuredMeasure>,
}

/// A submission that could not be stored, and why — surfaced so the worker (and a
/// person reading the run) sees what was dropped rather than a silent miss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejection {
    pub source: String,
    pub id: String,
    pub reason: String,
}

/// What a `push` did.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct SubmitReport {
    /// Recipes whose readings were stored.
    pub accepted: usize,
    /// Recipes re-derived so their readings show in `recipes` now.
    pub derived: usize,
    /// Submissions dropped, with the reason.
    pub rejected: Vec<Rejection>,
}

/// Store a batch of the worker's readings, then re-derive the accepted recipes.
///
/// Each submission is validated before it is stored: the recipe must still exist,
/// and its reading count must match the recipe's **current** ingredient list. A
/// mismatch means the raw changed between the worker's pull and its push, so the
/// readings would misalign — it is rejected (the recipe re-enters [`pending`] and
/// is read again) rather than stored wrong. Everything accepted is stored under one
/// `run_id` (the clobber guard, #11), then re-derived so `recipes` shows the
/// readings at once, without replaying the whole corpus.
pub async fn submit(
    conn: &Connection,
    items: Vec<SubmittedReadings>,
    model: &str,
    run_id: i64,
) -> anyhow::Result<SubmitReport> {
    let mut report = SubmitReport::default();
    let mut accepted: Vec<RecipeKey> = Vec::new();

    for item in items {
        match current_ingredient_count(conn, &item.source, &item.id).await? {
            Some(count) if count == item.readings.len() => {
                store(conn, &item.source, &item.id, &item.readings, model, run_id).await?;
                accepted.push((item.source, item.id));
                report.accepted += 1;
            }
            Some(count) => report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!(
                    "reading count {} does not match the recipe's {count} ingredients \
                     (raw changed since pull?)",
                    item.readings.len()
                ),
            }),
            None => report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "no such recipe".into(),
            }),
        }
    }

    if !accepted.is_empty() {
        report.derived = derive::derive_recipes(conn, &accepted, run_id)
            .await?
            .derived;
    }
    Ok(report)
}

/// The number of ingredients a recipe currently has, or `None` if there is no such
/// recipe — the count a submission's readings must match.
async fn current_ingredient_count(
    conn: &Connection,
    source: &str,
    id: &str,
) -> anyhow::Result<Option<usize>> {
    let mut rows = conn
        .query(
            "SELECT ingredients FROM recipes WHERE source = ?1 AND id = ?2",
            libsql::params![source.to_owned(), id.to_owned()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let json: String = row.get(0)?;
    let ingredients: Vec<Ingredient> = serde_json::from_str(&json).unwrap_or_default();
    Ok(Some(ingredients.len()))
}

// --- Storage + the derive-time join. -------------------------------------------

/// Load every recipe's readings into a map so [`crate::derive`] can reattach in
/// memory — one query, not a lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Vec<StructuredMeasure>>> {
    let mut rows = conn
        .query(
            "SELECT source, id, structured FROM ingredient_structures",
            (),
        )
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let structured: String = row.get(2)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal.
        if let Ok(readings) = serde_json::from_str::<Vec<StructuredMeasure>>(&structured) {
            map.insert((source, id), readings);
        }
    }
    Ok(map)
}

/// Reattach a recipe's readings onto its ingredients in place — the join `derive`
/// performs, offline. Attaches only when the stored array still lines up with the
/// recipe's ingredients (same count): a reading left over from a since-changed raw
/// simply doesn't attach (the recipe re-enriches next run) rather than misaligning.
/// A recipe with no row keeps `structured: None` — raw stays the source of truth.
pub fn attach(
    readings_by_recipe: &HashMap<RecipeKey, Vec<StructuredMeasure>>,
    source: &str,
    id: &str,
    ingredients: &mut [Ingredient],
) {
    let Some(readings) = readings_by_recipe.get(&(source.to_owned(), id.to_owned())) else {
        return;
    };
    if readings.len() != ingredients.len() {
        return;
    }
    for (ing, reading) in ingredients.iter_mut().zip(readings) {
        ing.structured = Some(reading.clone());
    }
}

/// Write one recipe's readings, keyed by `(source, id)`, stamped with the model and
/// the run. The `run_id` guard (`WHERE excluded.run_id >=
/// ingredient_structures.run_id`) stops a stale or partial run clobbering a newer
/// reading; a deliberate re-read overwrites under a fresh (higher) run id.
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    readings: &[StructuredMeasure],
    model: &str,
    run_id: i64,
) -> anyhow::Result<()> {
    let structured = serde_json::to_string(readings)?;
    conn.execute(
        "INSERT INTO ingredient_structures (source, id, structured, model, run_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source, id) DO UPDATE SET
            structured = excluded.structured,
            model      = excluded.model,
            created_at = unixepoch(),
            run_id     = excluded.run_id
         WHERE excluded.run_id >= ingredient_structures.run_id",
        libsql::params![source, id, structured, model, run_id],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(name: &str, measure: Option<&str>) -> Ingredient {
        Ingredient {
            name: name.into(),
            measure: measure.map(str::to_owned),
            structured: None,
        }
    }

    /// A reading of a line as just its item — enough to prove plumbing without
    /// modelling real extraction.
    fn item_reading(name: &str) -> StructuredMeasure {
        StructuredMeasure {
            item: name.into(),
            amount: None,
            preparation: None,
            note: None,
        }
    }

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    async fn insert_recipe(conn: &Connection, id: &str, ingredients: &[Ingredient]) {
        let json = serde_json::to_string(ingredients).unwrap();
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', ?1, 'T', ?2, 'go')",
            libsql::params![id, json],
        )
        .await
        .unwrap();
    }

    /// `pending` lists exactly the recipes with no reading yet, carrying their raw
    /// lines; an already-enriched recipe drops out.
    #[tokio::test]
    async fn pending_lists_unenriched_recipes_with_their_lines() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[ing("flour", Some("1 cup")), ing("salt", None)],
        )
        .await;
        insert_recipe(&conn, "2", &[ing("egg", Some("2"))]).await;
        // Recipe 2 already has a reading → it must not be pending.
        store(&conn, "themealdb", "2", &[item_reading("egg")], "m", 1)
            .await
            .unwrap();

        let p = pending(&conn, 25).await.unwrap();
        assert_eq!(p.len(), 1, "only the un-enriched recipe is pending");
        assert_eq!(p[0].id, "1");
        assert_eq!(p[0].ingredients.len(), 2);
        assert_eq!(p[0].ingredients[0].name, "flour");
        assert_eq!(p[0].ingredients[0].measure.as_deref(), Some("1 cup"));
        assert_eq!(p[0].ingredients[1].measure, None);
    }

    /// A recipe with no ingredients never earns a reading, so returning it would
    /// loop the worker forever — it must not appear in `pending`.
    #[tokio::test]
    async fn pending_excludes_empty_ingredient_recipes() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[]).await;
        assert!(
            pending(&conn, 25).await.unwrap().is_empty(),
            "an empty recipe must not be offered — it can never leave the queue"
        );
    }

    /// The limit bounds one pull's payload; the worker loops for the rest.
    #[tokio::test]
    async fn pending_respects_the_limit() {
        let conn = conn().await;
        for i in 0..5 {
            insert_recipe(&conn, &i.to_string(), &[ing("x", None)]).await;
        }
        assert_eq!(pending(&conn, 3).await.unwrap().len(), 3);
    }

    /// `submit` stores a matching submission, and rejects — never stores — one whose
    /// count no longer matches the recipe, or one for a recipe that does not exist.
    #[tokio::test]
    async fn submit_stores_matching_and_rejects_the_rest() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[ing("flour", Some("1 cup")), ing("salt", None)],
        )
        .await;
        insert_recipe(&conn, "2", &[ing("egg", Some("2"))]).await;

        let items = vec![
            // Matches recipe 1's two lines → accepted.
            SubmittedReadings {
                source: "themealdb".into(),
                id: "1".into(),
                readings: vec![item_reading("flour"), item_reading("salt")],
            },
            // Two readings for a one-ingredient recipe → rejected (raw changed).
            SubmittedReadings {
                source: "themealdb".into(),
                id: "2".into(),
                readings: vec![item_reading("a"), item_reading("b")],
            },
            // No such recipe → rejected.
            SubmittedReadings {
                source: "themealdb".into(),
                id: "9".into(),
                readings: vec![item_reading("x")],
            },
        ];

        let report = submit(&conn, items, "spy-model", 1).await.unwrap();
        assert_eq!(report.accepted, 1);
        assert_eq!(
            report.rejected.len(),
            2,
            "mismatch and unknown are both dropped"
        );

        let loaded = load(&conn).await.unwrap();
        let r1 = loaded.get(&("themealdb".into(), "1".into())).unwrap();
        assert_eq!(r1.len(), 2);
        assert_eq!(r1[0].item, "flour");
        assert!(
            loaded.get(&("themealdb".into(), "2".into())).is_none(),
            "a rejected submission stores nothing"
        );
    }

    /// Each stored row records which model produced it — provenance for a
    /// non-deterministic, drifting source.
    #[tokio::test]
    async fn submit_records_the_model_provenance() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[ing("flour", Some("1 cup"))]).await;
        submit(
            &conn,
            vec![SubmittedReadings {
                source: "themealdb".into(),
                id: "1".into(),
                readings: vec![item_reading("flour")],
            }],
            "claude-opus-4-8",
            1,
        )
        .await
        .unwrap();

        let mut rows = conn
            .query("SELECT model FROM ingredient_structures WHERE id = '1'", ())
            .await
            .unwrap();
        let model: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(model, "claude-opus-4-8");
    }

    /// The run-id guard on the writer: a stale run cannot clobber a newer reading; a
    /// higher run still can.
    #[tokio::test]
    async fn a_stale_run_cannot_clobber_a_reading() {
        let conn = conn().await;
        let read_item = |loaded: &HashMap<RecipeKey, Vec<StructuredMeasure>>| {
            loaded
                .get(&("themealdb".to_string(), "1".to_string()))
                .unwrap()[0]
                .item
                .clone()
        };

        store(&conn, "themealdb", "1", &[item_reading("run5")], "m", 5)
            .await
            .unwrap();
        // An older run writing late must be a no-op.
        store(&conn, "themealdb", "1", &[item_reading("run3")], "m", 3)
            .await
            .unwrap();
        assert_eq!(
            read_item(&load(&conn).await.unwrap()),
            "run5",
            "an older run must not clobber a newer reading"
        );

        // A newer run still wins.
        store(&conn, "themealdb", "1", &[item_reading("run9")], "m", 9)
            .await
            .unwrap();
        assert_eq!(read_item(&load(&conn).await.unwrap()), "run9");
    }

    /// `attach` is the join derive performs: a recipe's readings zip onto its
    /// ingredients; a recipe with no row stays `None`; a stored array whose count no
    /// longer matches (raw changed since) does not attach.
    #[test]
    fn attach_zips_matching_readings_and_leaves_the_rest() {
        let mut readings = HashMap::new();
        readings.insert(
            ("themealdb".to_string(), "1".to_string()),
            vec![item_reading("flour"), item_reading("salt")],
        );
        // A stale row for recipe 2: one reading, but the recipe has two ingredients.
        readings.insert(
            ("themealdb".to_string(), "2".to_string()),
            vec![item_reading("only one")],
        );

        let mut r1 = vec![ing("flour", Some("1 cup")), ing("salt", None)];
        attach(&readings, "themealdb", "1", &mut r1);
        assert_eq!(
            r1[0].structured.as_ref().map(|m| &m.item),
            Some(&"flour".to_string())
        );
        assert_eq!(
            r1[1].structured.as_ref().map(|m| &m.item),
            Some(&"salt".to_string())
        );

        // Count mismatch → nothing attaches (re-enriches next run rather than misalign).
        let mut r2 = vec![ing("a", None), ing("b", None)];
        attach(&readings, "themealdb", "2", &mut r2);
        assert!(r2.iter().all(|i| i.structured.is_none()));

        // No row at all → None.
        let mut r3 = vec![ing("x", None)];
        attach(&readings, "themealdb", "9", &mut r3);
        assert_eq!(r3[0].structured, None);
    }
}
