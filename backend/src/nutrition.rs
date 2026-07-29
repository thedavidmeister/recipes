//! The nutrition reading (#162): what a recipe costs you in kcal, read off the service.
//!
//! The fourth enrichment, and the same pipeline as the three before it: the app offers
//! recipes that have not been read, a worker pulls them, a model reads them, and the
//! app validates and writes every reading. No model code, prompt or provider key lives
//! here (#59).
//!
//! What is different is **how little the model is asked for**. A recipe's energy is
//! almost entirely arithmetic, and CLAUDE.md forbids asking a model to add up. So the
//! reading is two facts *about each food* — its energy density, and (only where a unit
//! table cannot say) the mass of one of the line's unit — and every multiplication and
//! sum happens in `recipe_core::nutrition`. The model never produces a number that
//! grows with the recipe, which is exactly why the result can be checked: "how many
//! kcal in 100 g of butter" has a right answer, "how many calories in this lasagne"
//! does not.
//!
//! **This queue runs behind the ingredient queue.** Energy needs a quantity, and the
//! quantity comes from the #11 reading. [`pending`] therefore offers only recipes whose
//! ingredient lines have already been read into structure — a recipe read for nutrition
//! first would store a reading that sums to nothing.

use std::collections::HashMap;

use libsql::Connection;
use recipe_core::{nutrition, FoodEnergy, Ingredient};
use serde::{Deserialize, Serialize};

use crate::{derive, runs};

type RecipeKey = (String, String);

/// What is stored for one recipe: the per-ingredient reading and the serving count.
type Reading = (Vec<FoodEnergy>, u32);

// --- The pull side: what still needs reading. ----------------------------------

/// One recipe awaiting a nutrition reading.
///
/// It carries more context than the other queues because it asks a question about the
/// whole dish as well as about each line: `title` and `instructions` are what a person
/// reads to say "this serves four" — a 9x13 dish, "divide between bowls", 500 g of
/// pasta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingNutritionRecipe {
    pub source: String,
    pub id: String,
    pub title: String,
    pub instructions: String,
    pub ingredients: Vec<PendingNutritionIngredient>,
}

/// One ingredient line as the nutrition reading sees it: the raw text, the food the
/// #11 reading named, and the unit a gram weight would have to answer for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingNutritionIngredient {
    pub name: String,
    pub measure: Option<String>,
    /// The food as the ingredient reading read it — `"flour"` out of `"1 cup flour"`.
    pub item: Option<String>,
    /// The line's unit, which is what [`FoodEnergy::grams_per_unit`] must be *per*.
    /// `None` for a bare count (`"2 eggs"`) — one egg is still a unit.
    pub unit: Option<String>,
    /// Whether the app can already weigh this line without the reading's help.
    ///
    /// Computed here, by the same ladder that does the summing, so the worker is asked
    /// for a gram weight only where deterministic code has no answer — and is never
    /// invited to second-guess one it does. `true` means: give the density, skip the
    /// weight.
    pub weighable: bool,
}

/// Recipes whose ingredient lines have been read but whose nutrition has not, capped at
/// `limit`.
///
/// Three conditions, and the third is the one this queue adds. The recipe must have
/// ingredients at all; it must have no nutrition reading yet; and **at least one of its
/// ingredient lines must already carry a structured measure**. Without that last one
/// the worker would be handed recipes it cannot usefully read — there would be no
/// quantity to multiply — and, worse, it would read them, the app would store the
/// reading, and the recipe would leave the queue permanently with a total of nothing.
///
/// The check is in SQL rather than a filter after the `LIMIT`, for the same reason
/// every other queue puts its exclusions there: a page of rows that Rust then discards
/// comes back as an empty array, and the worker's "loop until pending is empty" would
/// stop with the queue full.
pub async fn pending(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<PendingNutritionRecipe>> {
    let limit = limit.max(1) as i64;
    let mut rows = conn
        .query(
            "SELECT r.source, r.id, r.title, r.instructions, r.ingredients
             FROM recipes r
             LEFT JOIN nutrition_structures n ON n.source = r.source AND n.id = r.id
             WHERE n.id IS NULL
               AND json_valid(r.ingredients)
               AND json_array_length(r.ingredients) > 0
               AND EXISTS (
                   SELECT 1 FROM json_each(r.ingredients) line
                   WHERE json_extract(line.value, '$.structured') IS NOT NULL
               )
             LIMIT ?1",
            libsql::params![limit],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let title: String = row.get(2)?;
        let instructions: Option<String> = row.get(3)?;
        let json: String = row.get(4)?;
        // The SQL already required a JSON array with a structured line in it, so a
        // parse failure here is not expected — skip rather than fail the whole pull.
        let ingredients: Vec<Ingredient> = serde_json::from_str(&json).unwrap_or_default();
        if ingredients.is_empty() {
            continue;
        }
        out.push(PendingNutritionRecipe {
            source,
            id,
            title,
            instructions: instructions.unwrap_or_default(),
            ingredients: ingredients
                .into_iter()
                .map(|i| {
                    let structured = i.structured;
                    PendingNutritionIngredient {
                        name: i.name,
                        measure: i.measure,
                        item: structured.as_ref().map(|s| s.item.clone()),
                        unit: structured.as_ref().and_then(unit_of),
                        weighable: structured
                            .as_ref()
                            .is_some_and(nutrition::weighable_without_reading),
                    }
                })
                .collect(),
        });
    }
    Ok(out)
}

/// The unit a gram weight would have to be per, or `None` for a bare count or a line
/// with no number.
fn unit_of(measure: &recipe_core::StructuredMeasure) -> Option<String> {
    match measure.amount.as_ref()? {
        recipe_core::Amount::Quantified { unit, .. } => unit.clone(),
        recipe_core::Amount::Qualitative { .. } => None,
    }
}

// --- The push side: the worker's readings. -------------------------------------

/// One recipe's nutrition reading as the worker submits it: the foods in ingredient
/// order, plus how many people it feeds. Provenance (the model) is stamped once per
/// batch by `push`, not carried here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubmittedNutrition {
    pub source: String,
    pub id: String,
    pub foods: Vec<FoodEnergy>,
    pub servings: u32,
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
/// Validated on the way in, like every other push: the recipe must still exist, the
/// food count must match its **current** ingredient list (the raw may have changed
/// since the pull, and a misaligned reading sums to a plausible number about a
/// different recipe), and every figure must be one a food could actually have
/// ([`nutrition::validate`]).
///
/// The derive run is allocated **after** storage, so a concurrent ingest that derived
/// `recipes` first cannot leave an accepted reading unattached — the same reasoning as
/// [`crate::enrich::submit`].
pub async fn submit(
    conn: &Connection,
    items: Vec<SubmittedNutrition>,
    model: &str,
) -> anyhow::Result<SubmitReport> {
    let store_run = runs::begin(conn, "enrich_nutrition").await?;
    let (mut report, accepted) = match store_batch(conn, items, model, store_run).await {
        Ok(stored) => stored,
        // A run that errored out closes itself `failed` (#174), the same as the other
        // three enrichments. Abandoning the row would file this alongside the process
        // Render killed mid-flight, and those are two different facts.
        Err(e) => return Err(runs::fail(conn, store_run, e).await),
    };
    // A reading the worker offered that this run did not store is work it was handed
    // and dropped — an unknown recipe, a count that moved under it, a figure no food
    // could have, a race it lost. Each is named in `rejected`; the status is what
    // points a person at them. A batch stored whole is the only `completed` one.
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
    items: Vec<SubmittedNutrition>,
    model: &str,
    store_run: i64,
) -> anyhow::Result<(SubmitReport, Vec<RecipeKey>)> {
    let mut report = SubmitReport::default();
    let mut accepted: Vec<RecipeKey> = Vec::new();
    for item in items {
        let Some(count) = current_ingredient_count(conn, &item.source, &item.id).await? else {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "no such recipe".into(),
            });
            continue;
        };
        if count != item.foods.len() {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!(
                    "reading count {} does not match the recipe's {count} ingredients",
                    item.foods.len()
                ),
            });
            continue;
        }
        if let Err(reason) = nutrition::validate(&item.foods, item.servings) {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!("invalid nutrition reading: {reason}"),
            });
            continue;
        }
        let wrote = store(
            conn,
            &item.source,
            &item.id,
            &item.foods,
            item.servings,
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

/// How many ingredients the recipe has **right now**, or `None` if it is gone. The
/// count the submitted reading has to match.
async fn current_ingredient_count(
    conn: &Connection,
    source: &str,
    id: &str,
) -> anyhow::Result<Option<usize>> {
    let mut rows = conn
        .query(
            "SELECT json_array_length(ingredients) FROM recipes WHERE source = ?1 AND id = ?2",
            libsql::params![source.to_owned(), id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<i64>(0)? as usize)),
        None => Ok(None),
    }
}

// --- Storage, and the join `derive` performs. ----------------------------------

/// Write one reading, guarded on `run_id` so a stale run cannot clobber a newer one.
#[allow(clippy::too_many_arguments)]
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    foods: &[FoodEnergy],
    servings: u32,
    model: &str,
    run_id: i64,
) -> anyhow::Result<bool> {
    let structured = serde_json::to_string(foods)?;
    let affected = conn
        .execute(
            "INSERT INTO nutrition_structures (source, id, structured, servings, model, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source, id) DO UPDATE SET
                structured = excluded.structured,
                servings   = excluded.servings,
                model      = excluded.model,
                created_at = unixepoch(),
                run_id     = excluded.run_id
             WHERE excluded.run_id >= nutrition_structures.run_id",
            libsql::params![source, id, structured, i64::from(servings), model, run_id],
        )
        .await?;
    Ok(affected > 0)
}

/// Load every reading so [`crate::derive`] can reattach in memory — one query, not a
/// lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Reading>> {
    let mut rows = conn
        .query(
            "SELECT source, id, structured, servings FROM nutrition_structures",
            (),
        )
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let structured: String = row.get(2)?;
        let servings: i64 = row.get(3)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal.
        if let Ok(foods) = serde_json::from_str::<Vec<FoodEnergy>>(&structured) {
            map.insert((source, id), (foods, servings.max(0) as u32));
        }
    }
    Ok(map)
}

/// Reattach a recipe's reading in place. A recipe with no row keeps `[]`/`None` —
/// "not known yet", the degrade-not-die state.
pub fn attach(
    by_recipe: &HashMap<RecipeKey, Reading>,
    source: &str,
    id: &str,
    nutrition: &mut Vec<FoodEnergy>,
    servings: &mut Option<u32>,
) {
    if let Some((foods, read_servings)) = by_recipe.get(&(source.to_owned(), id.to_owned())) {
        *nutrition = foods.clone();
        *servings = Some(*read_servings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::{Amount, Quantity, StructuredMeasure};

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    fn line(name: &str, unit: Option<&str>, read: bool) -> Ingredient {
        Ingredient {
            name: name.into(),
            measure: Some("1".into()),
            structured: read.then(|| StructuredMeasure {
                item: name.to_lowercase(),
                amount: Some(Amount::Quantified {
                    quantity: Quantity::Exact { value: 1.0 },
                    unit: unit.map(str::to_owned),
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        }
    }

    async fn insert_recipe(conn: &Connection, id: &str, ingredients: &[Ingredient]) {
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', ?1, 'Stew', ?2, 'Cook it.')",
            libsql::params![id, serde_json::to_string(ingredients).unwrap()],
        )
        .await
        .unwrap();
    }

    fn food(kcal_per_100g: f64, grams_per_unit: Option<f64>) -> FoodEnergy {
        FoodEnergy {
            kcal_per_100g,
            grams_per_unit,
        }
    }

    fn submitted(id: &str, foods: Vec<FoodEnergy>, servings: u32) -> SubmittedNutrition {
        SubmittedNutrition {
            source: "themealdb".into(),
            id: id.into(),
            foods,
            servings,
        }
    }

    /// The queue offers what has not been read, and stops offering it once it has.
    #[tokio::test]
    async fn pending_empties_as_readings_land() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;
        assert_eq!(pending(&conn, 10).await.unwrap().len(), 1);

        let report = submit(&conn, vec![submitted("1", vec![food(364.0, None)], 4)], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 1);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        assert!(
            pending(&conn, 10).await.unwrap().is_empty(),
            "a read recipe leaves the queue"
        );
    }

    /// **This queue runs behind the ingredient queue.** A recipe nobody has read for
    /// measures has no quantity to multiply, so offering it would earn a stored reading
    /// that sums to nothing and drop it from the queue for good.
    #[tokio::test]
    async fn a_recipe_with_no_ingredient_reading_is_never_offered() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), false)]).await;
        assert!(pending(&conn, 10).await.unwrap().is_empty());

        // Once its measures are read, it appears.
        conn.execute(
            "UPDATE recipes SET ingredients = ?1 WHERE id = '1'",
            libsql::params![serde_json::to_string(&[line("Flour", Some("g"), true)]).unwrap()],
        )
        .await
        .unwrap();
        assert_eq!(pending(&conn, 10).await.unwrap().len(), 1);
    }

    /// A recipe with no ingredients at all can never earn a reading, so it must never
    /// be offered — the worker reads until `pending` is empty.
    #[tokio::test]
    async fn a_recipe_with_no_ingredients_is_never_offered() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[]).await;
        assert!(pending(&conn, 10).await.unwrap().is_empty());
    }

    /// The pull tells the worker which lines it need not weigh, computed by the same
    /// ladder that does the summing. Grams are code's; cups and cloves are the model's.
    #[tokio::test]
    async fn the_pull_says_which_lines_the_app_can_already_weigh() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[
                line("Chicken", Some("g"), true),
                line("Flour", Some("cup"), true),
                line("Egg", None, true),
            ],
        )
        .await;

        let pulled = pending(&conn, 10).await.unwrap();
        let lines = &pulled[0].ingredients;
        assert_eq!(lines[0].unit.as_deref(), Some("g"));
        assert!(lines[0].weighable, "grams need nothing from the model");
        assert_eq!(lines[1].unit.as_deref(), Some("cup"));
        assert!(!lines[1].weighable, "a cup of what? the model must say");
        assert_eq!(
            lines[2].unit, None,
            "a bare count still has a unit: one egg"
        );
        assert!(!lines[2].weighable);

        // The dish-level context the servings reading needs rides along.
        assert_eq!(pulled[0].title, "Stew");
        assert_eq!(pulled[0].instructions, "Cook it.");
        assert_eq!(lines[0].item.as_deref(), Some("chicken"));
    }

    /// A reading that has drifted out of alignment with the recipe is refused, not
    /// stored against the wrong lines — the recipe re-enters the queue.
    #[tokio::test]
    async fn a_count_mismatch_is_refused_and_the_recipe_comes_back() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            &[line("Flour", Some("g"), true), line("Egg", None, true)],
        )
        .await;

        let report = submit(&conn, vec![submitted("1", vec![food(364.0, None)], 4)], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(
            report.rejected[0].reason.contains("does not match"),
            "{}",
            report.rejected[0].reason
        );
        assert_eq!(pending(&conn, 10).await.unwrap().len(), 1);
    }

    /// An impossible figure is refused rather than summed into a total nobody can
    /// audit — 2,400 kcal/100 g is a whole recipe wearing a density's clothes.
    #[tokio::test]
    async fn an_impossible_reading_is_refused() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;

        let report = submit(
            &conn,
            vec![submitted("1", vec![food(2400.0, None)], 4)],
            "m",
        )
        .await
        .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("nothing edible exceeds"));

        // And so is a serving count no home recipe means.
        let report = submit(&conn, vec![submitted("1", vec![food(364.0, None)], 0)], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("servings is 0"));
    }

    /// A reading for a recipe we do not have is dropped rather than stored against
    /// nothing.
    #[tokio::test]
    async fn a_reading_for_an_unknown_recipe_is_dropped() {
        let conn = conn().await;
        let report = submit(&conn, vec![submitted("nope", vec![], 4)], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.rejected[0].reason, "no such recipe");
    }

    /// The status a run row ended on — the fourth enrichment has to answer #174 the
    /// same way the other three do.
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
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;
        let report = submit(&conn, vec![submitted("1", vec![food(364.0, None)], 4)], "m")
            .await
            .unwrap();
        assert!(report.rejected.is_empty());
        assert_eq!(
            last_run_status(&conn, "enrich_nutrition").await,
            runs::COMPLETED,
            "a batch stored whole is completed"
        );
    }

    /// A push that dropped a reading is `partial`, never `completed` (#174). The run
    /// finished, but it did not do everything it was handed, and `rejected` says which
    /// — the status is what points a person at a worker sending nonsense.
    #[tokio::test]
    async fn a_push_that_dropped_a_reading_records_partial() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;
        let report = submit(
            &conn,
            vec![
                submitted("1", vec![food(364.0, None)], 4),
                // No such recipe → rejected, so this batch was not stored whole.
                submitted("9", vec![food(100.0, None)], 4),
            ],
            "m",
        )
        .await
        .unwrap();

        assert_eq!(report.accepted, 1);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(
            last_run_status(&conn, "enrich_nutrition").await,
            runs::PARTIAL,
            "a dropped reading is work the run was handed and did not do"
        );
    }

    /// A figure no food could have is rejected, and that too makes the run `partial`
    /// rather than clean — an invalid reading is dropped work like any other.
    #[tokio::test]
    async fn an_impossible_figure_makes_the_run_partial() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;
        let report = submit(
            &conn,
            vec![submitted("1", vec![food(99_999.0, None)], 4)],
            "m",
        )
        .await
        .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(
            last_run_status(&conn, "enrich_nutrition").await,
            runs::PARTIAL
        );
    }

    /// A stale run cannot overwrite a newer reading, and the push says so rather than
    /// reporting a write that never happened.
    #[tokio::test]
    async fn a_superseded_write_is_reported_not_counted() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;

        // A reading written by a run far in the future.
        assert!(
            store(&conn, "themealdb", "1", &[food(364.0, None)], 4, "m", 9_999)
                .await
                .unwrap()
        );

        let report = submit(&conn, vec![submitted("1", vec![food(1.0, None)], 2)], "m")
            .await
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert!(report.rejected[0].reason.contains("superseded"));

        let (foods, servings) =
            load(&conn).await.unwrap()[&("themealdb".to_string(), "1".to_string())].clone();
        assert_eq!(foods, vec![food(364.0, None)], "the newer reading stands");
        assert_eq!(servings, 4);
    }

    /// `load` and `attach` are the join `derive` performs: a recipe with a reading gets
    /// both halves of it, and one without keeps the unread state rather than a guess.
    #[tokio::test]
    async fn attach_fills_a_read_recipe_and_leaves_an_unread_one_alone() {
        let conn = conn().await;
        insert_recipe(&conn, "1", &[line("Flour", Some("g"), true)]).await;
        submit(&conn, vec![submitted("1", vec![food(364.0, None)], 6)], "m")
            .await
            .unwrap();
        let readings = load(&conn).await.unwrap();

        let mut foods = Vec::new();
        let mut servings = None;
        attach(&readings, "themealdb", "1", &mut foods, &mut servings);
        assert_eq!(foods, vec![food(364.0, None)]);
        assert_eq!(servings, Some(6));

        let mut foods = Vec::new();
        let mut servings = None;
        attach(&readings, "themealdb", "unread", &mut foods, &mut servings);
        assert!(foods.is_empty());
        assert_eq!(servings, None, "unread stays unread, never a default 1");
    }
}
