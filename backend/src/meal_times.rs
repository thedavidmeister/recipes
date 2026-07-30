//! The meal-time reading (#191): when each dish is eaten, read off the service.
//!
//! The fifth enrichment, and the same pipeline as the four before it: the app offers
//! recipes that have not been read, a worker pulls them, a model reads them, and the app
//! validates and writes every reading. No model code, prompt or provider key lives here
//! (#59).
//!
//! What is different is that this reading exists to make a control *work*. A plan asks
//! "which meal", and until this table has rows there is no fact in the corpus that could
//! tell breakfast from dinner — the corpus carries no `Lunch`, `Dinner` or `Snack`
//! category at all, and 19 of 790 recipes carry `Breakfast`. #188 narrowed a meal round
//! as far as stated data allows and said so; this is what takes it the rest of the way.
//!
//! **The reading is a set** ([`recipe_core::meal::Sitting`]) — a chicken curry is lunch
//! or dinner, a roast is dinner — and it is **never empty**: every dish is eaten at some
//! time, so an empty set is a failed reading rather than a fact about the food, and it is
//! refused on the way in beside the other rejections. That refusal is what lets an empty
//! set mean *unread* everywhere downstream.
//!
//! **This queue runs beside the others, not behind them.** Unlike nutrition, which needs
//! the ingredient reading's quantities to mean anything, when a dish is eaten can be read
//! from its title and method alone — so nothing is gated on another queue, and a recipe
//! is offered as soon as it exists.

use std::collections::HashMap;

use libsql::Connection;
use recipe_core::{meal, Sitting};
use serde::{Deserialize, Serialize};

use crate::{derive, runs};

type RecipeKey = (String, String);

// --- The pull side: what still needs reading. ----------------------------------

/// One recipe awaiting a meal-time reading.
///
/// It carries what a person would actually look at to answer "when do you eat this":
/// the dish's name first, then its category and cuisine (a `Dessert` is eaten at a
/// different point in the day than a `Breakfast`), its method, and its ingredients. The
/// ingredients matter more here than the count of them does — eggs and oats read
/// differently from a joint of beef — so they ride as plain names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingMealTimeRecipe {
    pub source: String,
    pub id: String,
    pub title: String,
    /// The source's own word for the dish (`Dessert`, `Beef`, `Breakfast`), exactly as
    /// stored. It is a hint and not an answer — that is the whole finding of #188 — but
    /// where the source *did* state something, the reading should not contradict it
    /// without reason.
    pub category: Option<String>,
    /// The cuisine, which carries real signal about when a dish is eaten: a full English
    /// and a congee are both breakfasts in their own kitchens.
    pub area: Option<String>,
    pub instructions: String,
    pub ingredients: Vec<String>,
}

/// Recipes with no meal-time reading yet, capped at `limit`.
///
/// One condition beyond "not yet read": the recipe must have a title. A row with a blank
/// title is a partial that never resolved into a dish, and there is nothing to read a
/// sitting from — offering one would keep it in the queue forever, because the worker
/// reads until `pending` is empty.
///
/// The check is in SQL rather than a filter after the `LIMIT`, for the same reason every
/// other queue puts its exclusions there: a page of rows that Rust then discards comes
/// back as an empty array, and the worker's "loop until pending is empty" would stop with
/// the queue full.
///
/// **Accompaniments are offered like everything else.** A dessert is eaten at a sitting
/// too, and #147's per-addition rounds will ask this table exactly that question; the
/// meal round's exclusion of them (#188) is a rule about the *round*, not a claim that a
/// trifle is never eaten.
pub async fn pending(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<PendingMealTimeRecipe>> {
    let limit = limit.max(1) as i64;
    let mut rows = conn
        .query(
            "SELECT r.source, r.id, r.title, r.category, r.area, r.instructions, r.ingredients
             FROM recipes r
             LEFT JOIN meal_time_structures m ON m.source = r.source AND m.id = r.id
             WHERE m.id IS NULL
               AND trim(r.title) <> ''
             LIMIT ?1",
            libsql::params![limit],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let json: String = row.get(6)?;
        // `ingredients` is our own serialization, NOT NULL DEFAULT '[]'. A row that no
        // longer parses degrades to no ingredients rather than failing the whole pull —
        // the title, category and method are still enough to read a sitting from.
        let ingredients: Vec<recipe_core::Ingredient> =
            serde_json::from_str(&json).unwrap_or_default();
        out.push(PendingMealTimeRecipe {
            source: row.get::<String>(0)?,
            id: row.get::<String>(1)?,
            title: row.get::<String>(2)?,
            category: row.get::<Option<String>>(3)?,
            area: row.get::<Option<String>>(4)?,
            instructions: row.get::<Option<String>>(5)?.unwrap_or_default(),
            ingredients: ingredients.into_iter().map(|i| i.name).collect(),
        });
    }
    Ok(out)
}

// --- The push side: the worker's readings. -------------------------------------

/// One recipe's meal-time reading as the worker submits it. Provenance (the model) is
/// stamped once per batch by `push`, not carried here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubmittedMealTimes {
    pub source: String,
    pub id: String,
    /// The sittings this dish suits. Serde refuses a word outside the vocabulary at the
    /// wire, so by the time this exists every entry is one of the four; what still has to
    /// be checked is that there is at least one of them and no repeats.
    pub sittings: Vec<Sitting>,
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
/// Validated on the way in, like every other push: the recipe must still exist, and the
/// set must be one a dish could have ([`meal::validate`] — non-empty, no repeats).
///
/// There is no count check here, because unlike the ingredient, step and nutrition
/// readings this one is not aligned to anything: it is a claim about the whole dish, so
/// there is nothing for the raw to have drifted out of step with. What replaces it is
/// the vocabulary itself — a closed enum serde refuses to widen.
///
/// The derive run is allocated **after** storage, so a concurrent ingest that derived
/// `recipes` first cannot leave an accepted reading unattached — the same reasoning as
/// [`crate::enrich::submit`].
pub async fn submit(
    conn: &Connection,
    items: Vec<SubmittedMealTimes>,
    model: &str,
) -> anyhow::Result<SubmitReport> {
    let store_run = runs::begin(conn, "enrich_meal_times").await?;
    let (mut report, accepted) = match store_batch(conn, items, model, store_run).await {
        Ok(stored) => stored,
        // A run that errored out closes itself `failed` (#174), the same as the other
        // four enrichments. Abandoning the row would file this alongside the process
        // Render killed mid-flight, and those are two different facts.
        Err(e) => return Err(runs::fail(conn, store_run, e).await),
    };
    // A reading the worker offered that this run did not store is work it was handed and
    // dropped — an unknown recipe, an empty or repeating set, a race it lost. Each is
    // named in `rejected`; the status is what points a person at them. A batch stored
    // whole is the only `completed` one.
    let store_outcome = if report.rejected.is_empty() {
        runs::Outcome::Completed
    } else {
        runs::Outcome::Partial
    };
    runs::finish(conn, store_run, store_outcome).await?;

    // The derive run is allocated *here*, after storage — see the doc comment.
    if !accepted.is_empty() {
        let derive_run = runs::begin(conn, "derive").await?;
        match derive::derive_recipes(conn, &accepted, derive_run).await {
            Ok(derived) => {
                report.derived = derived.derived;
                runs::finish(conn, derive_run, runs::Outcome::Completed).await?;
            }
            Err(e) => return Err(runs::fail(conn, derive_run, e).await),
        }
    }
    Ok(report)
}

/// Validate and store one batch under `store_run`, returning what happened and the
/// recipes worth re-deriving.
///
/// Separated from [`submit`] so an error anywhere in the batch has one exit, and
/// [`submit`] can close the run [`runs::Outcome::Failed`] on the way out (#174).
async fn store_batch(
    conn: &Connection,
    items: Vec<SubmittedMealTimes>,
    model: &str,
    store_run: i64,
) -> anyhow::Result<(SubmitReport, Vec<RecipeKey>)> {
    let mut report = SubmitReport::default();
    let mut accepted: Vec<RecipeKey> = Vec::new();
    for item in items {
        if !recipe_exists(conn, &item.source, &item.id).await? {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "no such recipe".into(),
            });
            continue;
        }
        if let Err(reason) = meal::validate(&item.sittings) {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!("invalid meal-time reading: {reason}"),
            });
            continue;
        }
        // Stored as a set, in vocabulary order: two spellings of one fact are one row,
        // so a re-read that answered "dinner, lunch" does not look like a changed
        // reading. `validate` has already refused the one reordering could hide — a
        // repeat.
        let wrote = store(
            conn,
            &item.source,
            &item.id,
            &meal::canonical(&item.sittings),
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
    Ok((report, accepted))
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

// --- Storage, and the join `derive` performs. ----------------------------------

/// Write one reading, guarded on `run_id` so a stale run cannot clobber a newer one.
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    sittings: &[Sitting],
    model: &str,
    run_id: i64,
) -> anyhow::Result<bool> {
    let sittings = serde_json::to_string(sittings)?;
    let affected = conn
        .execute(
            "INSERT INTO meal_time_structures (source, id, sittings, model, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source, id) DO UPDATE SET
                sittings   = excluded.sittings,
                model      = excluded.model,
                created_at = unixepoch(),
                run_id     = excluded.run_id
             WHERE excluded.run_id >= meal_time_structures.run_id",
            libsql::params![source, id, sittings, model, run_id],
        )
        .await?;
    Ok(affected > 0)
}

/// Load every reading so [`crate::derive`] can reattach in memory — one query, not a
/// lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Vec<Sitting>>> {
    let mut rows = conn
        .query("SELECT source, id, sittings FROM meal_time_structures", ())
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let sittings: String = row.get(2)?;
        // A row that no longer deserializes (a shape change, or a word retired from the
        // vocabulary) is skipped, not fatal — it reads as unread, which restricts
        // nothing, rather than failing every derive.
        if let Ok(sittings) = serde_json::from_str::<Vec<Sitting>>(&sittings) {
            map.insert((source, id), sittings);
        }
    }
    Ok(map)
}

/// Reattach a recipe's reading in place. A recipe with no row keeps `[]` — "not known
/// yet", the degrade-not-die state that [`meal::fit`] reads as unrestricted.
pub fn attach(
    by_recipe: &HashMap<RecipeKey, Vec<Sitting>>,
    source: &str,
    id: &str,
    sittings: &mut Vec<Sitting>,
) {
    if let Some(read) = by_recipe.get(&(source.to_owned(), id.to_owned())) {
        *sittings = read.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::Sitting::{Breakfast, Dinner, Lunch, Snack};

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    async fn insert_recipe(conn: &Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO recipes (source, id, title, category, area, instructions, ingredients)
             VALUES ('themealdb', ?1, ?2, 'Chicken', 'Indian', 'Simmer it.',
                     '[{\"name\":\"chicken\",\"measure\":\"500 g\"}]')",
            libsql::params![id, title],
        )
        .await
        .unwrap();
    }

    fn submitted(id: &str, sittings: Vec<Sitting>) -> SubmittedMealTimes {
        SubmittedMealTimes {
            source: "themealdb".into(),
            id: id.into(),
            sittings,
        }
    }

    async fn stored(conn: &Connection, id: &str) -> Vec<Sitting> {
        load(conn)
            .await
            .unwrap()
            .get(&("themealdb".to_string(), id.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// The queue offers what has not been read, and stops offering it once it has.
    #[tokio::test]
    async fn pending_empties_as_readings_land() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;
        assert_eq!(pending(&conn, 10).await.unwrap().len(), 1);

        let report = submit(&conn, vec![submitted("1", vec![Lunch, Dinner])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 1);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        assert!(
            pending(&conn, 10).await.unwrap().is_empty(),
            "a read recipe leaves the queue"
        );
    }

    /// A recipe with no title is not a dish anyone can place in the day, and offering it
    /// would keep it in the queue forever — the worker reads until `pending` is empty.
    #[tokio::test]
    async fn a_recipe_with_no_title_is_never_offered() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "   ").await;
        assert!(pending(&conn, 10).await.unwrap().is_empty());
    }

    /// The pull carries what a person would actually look at to answer the question: the
    /// dish's name, the source's own word for it, the cuisine, the method and the
    /// ingredients.
    #[tokio::test]
    async fn the_pull_carries_what_the_reading_needs() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;

        let pulled = pending(&conn, 10).await.unwrap();
        assert_eq!(pulled[0].title, "Chicken curry");
        assert_eq!(pulled[0].category.as_deref(), Some("Chicken"));
        assert_eq!(pulled[0].area.as_deref(), Some("Indian"));
        assert_eq!(pulled[0].instructions, "Simmer it.");
        assert_eq!(pulled[0].ingredients, vec!["chicken".to_string()]);
    }

    /// **The rejection this reading turns on** (#158/#162's ruling applied again): an
    /// empty set is a failed reading, not a dish nobody eats. It is refused, so the
    /// recipe comes back on the next pull instead of leaving the queue permanently with a
    /// set no plan could ever match.
    #[tokio::test]
    async fn an_empty_set_is_refused_and_the_recipe_comes_back() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;

        let report = submit(&conn, vec![submitted("1", vec![])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(
            report.rejected[0]
                .reason
                .contains("every dish is eaten at some time"),
            "{}",
            report.rejected[0].reason
        );
        assert_eq!(
            pending(&conn, 10).await.unwrap().len(),
            1,
            "a refused reading leaves the recipe in the queue"
        );
    }

    /// A set that names the same sitting twice is a reading that misunderstood the
    /// question, so it is refused rather than quietly deduplicated.
    #[tokio::test]
    async fn a_repeated_sitting_is_refused() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;

        let report = submit(&conn, vec![submitted("1", vec![Dinner, Dinner])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("repeats"));
    }

    /// A word outside the vocabulary never reaches [`submit`] at all — the wire refuses
    /// it, because [`Sitting`] is a closed enum. That is why there is no "unknown word"
    /// rejection to test for below it.
    #[test]
    fn a_word_outside_the_vocabulary_is_refused_at_the_wire() {
        let good = r#"{"source":"themealdb","id":"1","sittings":["lunch","dinner"]}"#;
        assert_eq!(
            serde_json::from_str::<SubmittedMealTimes>(good).unwrap(),
            submitted("1", vec![Lunch, Dinner])
        );
        for bad in ["brunch", "supper", "Dinner", "elevenses", ""] {
            let json = format!(r#"{{"source":"themealdb","id":"1","sittings":["{bad}"]}}"#);
            assert!(
                serde_json::from_str::<SubmittedMealTimes>(&json).is_err(),
                "{bad:?} is not a sitting"
            );
        }
    }

    /// A set has no order, so the stored row is canonical however the model answered —
    /// otherwise a re-read that said "dinner, lunch" would look like a changed reading.
    #[tokio::test]
    async fn a_set_is_stored_in_vocabulary_order_however_it_arrived() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;
        insert_recipe(&conn, "2", "Toast").await;

        submit(
            &conn,
            vec![
                submitted("1", vec![Dinner, Lunch]),
                submitted("2", vec![Snack, Breakfast]),
            ],
            "m",
        )
        .await
        .unwrap();

        assert_eq!(stored(&conn, "1").await, vec![Lunch, Dinner]);
        assert_eq!(stored(&conn, "2").await, vec![Breakfast, Snack]);
    }

    /// A reading for a recipe we do not have is dropped rather than stored against
    /// nothing.
    #[tokio::test]
    async fn a_reading_for_an_unknown_recipe_is_dropped() {
        let conn = conn().await;
        let report = submit(&conn, vec![submitted("nope", vec![Dinner])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.rejected[0].reason, "no such recipe");
    }

    /// The status a run row ended on — the fifth enrichment has to answer #174 the same
    /// way the other four do.
    async fn last_run_status(conn: &Connection, kind: &str) -> String {
        let mut rows = conn
            .query(
                "SELECT status FROM runs WHERE kind = ?1 ORDER BY id DESC LIMIT 1",
                libsql::params![kind],
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<String>(0)
            .unwrap()
    }

    /// A push that stored everything it was handed is `completed`.
    #[tokio::test]
    async fn a_whole_push_records_completed() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;
        let report = submit(&conn, vec![submitted("1", vec![Lunch, Dinner])], "m")
            .await
            .unwrap();
        assert!(report.rejected.is_empty());
        assert_eq!(
            last_run_status(&conn, "enrich_meal_times").await,
            runs::COMPLETED,
            "a batch stored whole is completed"
        );
    }

    /// A push that dropped a reading is `partial`, never `completed` (#174). The run
    /// finished, but it did not do everything it was handed, and `rejected` says which.
    #[tokio::test]
    async fn a_push_that_dropped_a_reading_records_partial() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;
        let report = submit(
            &conn,
            vec![
                submitted("1", vec![Lunch, Dinner]),
                // An empty set → rejected, so this batch was not stored whole.
                submitted("1", vec![]),
            ],
            "m",
        )
        .await
        .unwrap();

        assert_eq!(report.accepted, 1);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(
            last_run_status(&conn, "enrich_meal_times").await,
            runs::PARTIAL,
            "a dropped reading is work the run was handed and did not do"
        );
    }

    /// A stale run cannot overwrite a newer reading, and the push says so rather than
    /// reporting a write that never happened.
    #[tokio::test]
    async fn a_superseded_write_is_reported_not_counted() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;

        // A reading written by a run far in the future.
        assert!(store(&conn, "themealdb", "1", &[Dinner], "m", 9_999)
            .await
            .unwrap());

        let report = submit(&conn, vec![submitted("1", vec![Breakfast])], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("superseded"));
        assert_eq!(
            stored(&conn, "1").await,
            vec![Dinner],
            "the newer reading stands"
        );
    }

    /// `load` and `attach` are the join `derive` performs: a recipe with a reading gets
    /// it, and one without keeps the unread state rather than a guess.
    #[tokio::test]
    async fn attach_fills_a_read_recipe_and_leaves_an_unread_one_alone() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chicken curry").await;
        submit(&conn, vec![submitted("1", vec![Lunch, Dinner])], "m")
            .await
            .unwrap();
        let readings = load(&conn).await.unwrap();

        let mut sittings = Vec::new();
        attach(&readings, "themealdb", "1", &mut sittings);
        assert_eq!(sittings, vec![Lunch, Dinner]);

        let mut sittings = Vec::new();
        attach(&readings, "themealdb", "unread", &mut sittings);
        assert!(
            sittings.is_empty(),
            "unread stays unread, never a default sitting"
        );
    }
}
