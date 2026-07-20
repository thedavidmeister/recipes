//! Step enrichment: the corpus's structured method readings (#74/#75/#76), and the
//! queue an off-Render worker uses to produce them.
//!
//! A reading turns a recipe's prose method into a [`StructuredStep`] DAG a GUI can
//! render as a graph: timers on timed steps (#74), parallel-vs-sequential from the
//! dependency edges (#75), and prep pulled out of an ingredient line into its own
//! step (#76). Producing one is an LLM job — reading messy prose into structure —
//! and, exactly as with the ingredient reading ([`crate::enrich`]), that job runs
//! **outside this app**: a worker pulls the work, a model reads, the worker pushes
//! results back through two machine-gated endpoints. The app holds no model code,
//! no prompt, and no provider credential, and stays the sole DB writer.
//!
//! The two ends of that queue, plus the storage between them:
//!
//! - [`pending`] — recipes with a method but no step reading yet, carrying the
//!   instructions to segment and the ingredients (with any preparation) to pull prep
//!   from. Served by [`crate::step_api::pending`] (`GET /api/enrich/steps/pending`).
//! - [`submit`] — a batch of the worker's step DAGs, validated (the recipe still
//!   exists, the graph is well-formed), stored, and re-derived. Driven by
//!   [`crate::step_api::results`] (`POST /api/enrich/steps/results`).
//! - [`store`]/[`load`]/[`attach`] — where a reading lands, and the join
//!   [`crate::derive`] performs to hang steps back onto `recipes`.
//!
//! **A capture, not a derivation** — its own table (`step_structures`), a peer of
//! `ingredient_structures`, per "each enrichment its own table". **Degrade-not-die**:
//! until the worker has read a recipe it carries `steps: []` and is not yet cookable
//! from the graph; the reading is an addition, never a gate on ingestion.

use std::collections::HashMap;

use libsql::Connection;
use recipe_core::{step, Ingredient, StructuredStep};
use serde::{Deserialize, Serialize};

use crate::{derive, runs};

/// A recipe key: `(source, id)`.
type RecipeKey = (String, String);

// --- The pull side: what still needs reading. ----------------------------------

/// One recipe awaiting a step reading: its key, the method to segment, and the
/// ingredients (with any preparation already read) so the model can pull hidden prep
/// out of an ingredient line into a step (#76).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingStepRecipe {
    pub source: String,
    pub id: String,
    pub instructions: String,
    pub ingredients: Vec<PendingStepIngredient>,
}

/// An ingredient as context for step reading — the raw line plus its structured
/// `preparation` if the ingredient enrichment has run (a chop/slice to extract, #76).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingStepIngredient {
    pub name: String,
    pub measure: Option<String>,
    pub preparation: Option<String>,
}

/// Recipes with a method but no step reading yet, capped at `limit`.
///
/// A left join for "no row in `step_structures`". Recipes with a blank method are
/// excluded (`trim(instructions) <> ''`): there is nothing to segment, and returning
/// one would loop the worker's "read until pending is empty" forever — a method-less
/// recipe never earns a reading, so it would never leave the queue.
pub async fn pending(conn: &Connection, limit: usize) -> anyhow::Result<Vec<PendingStepRecipe>> {
    let limit = limit.max(1) as i64;
    let mut rows = conn
        .query(
            "SELECT r.source, r.id, r.instructions, r.ingredients
             FROM recipes r
             LEFT JOIN step_structures s ON s.source = r.source AND s.id = r.id
             WHERE s.id IS NULL
               AND r.instructions IS NOT NULL
               AND trim(r.instructions) <> ''
             LIMIT ?1",
            libsql::params![limit],
        )
        .await?;

    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let instructions: String = row.get(2)?;
        let ing_json: String = row.get(3)?;
        // The ingredients ride along for prep extraction; a parse failure just omits
        // them (the method alone is still readable) rather than failing the pull.
        let ingredients: Vec<Ingredient> = serde_json::from_str(&ing_json).unwrap_or_default();
        out.push(PendingStepRecipe {
            source,
            id,
            instructions,
            ingredients: ingredients
                .into_iter()
                .map(|i| PendingStepIngredient {
                    name: i.name,
                    measure: i.measure,
                    preparation: i.structured.and_then(|s| s.preparation),
                })
                .collect(),
        });
    }
    Ok(out)
}

// --- The push side: the worker's step DAGs. ------------------------------------

/// One recipe's step reading as the worker submits it: the key and the DAG. The
/// model (provenance) is stamped by `push` from its environment, not carried here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubmittedSteps {
    pub source: String,
    pub id: String,
    pub steps: Vec<StructuredStep>,
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
    /// Recipes whose step reading was stored.
    pub accepted: usize,
    /// Recipes re-derived so their steps show in `recipes` now.
    pub derived: usize,
    /// Submissions dropped, with the reason.
    pub rejected: Vec<Rejection>,
}

/// Store a batch of the worker's step readings, then re-derive the accepted recipes.
///
/// Runs entirely server-side. Each submission is validated before storage: the
/// recipe must still exist, and the step DAG must be well-formed ([`step::validate`]
/// — 0-based sequential ids, every `after` edge pointing to an earlier step, so it is
/// acyclic by construction). A malformed graph is rejected (the recipe re-enters
/// [`pending`] and is read again) rather than stored wrong.
///
/// Two runs, deliberately — the same reasoning as the ingredient push
/// ([`crate::enrich::submit`]): store under one run, then derive the reattach under a
/// **fresh run allocated after storage**, so a concurrent ingest that derived
/// `recipes` first cannot leave the accepted reading unattached.
pub async fn submit(
    conn: &Connection,
    items: Vec<SubmittedSteps>,
    model: &str,
) -> anyhow::Result<SubmitReport> {
    let mut report = SubmitReport::default();
    let mut accepted: Vec<RecipeKey> = Vec::new();

    let store_run = runs::begin(conn, "enrich_steps").await?;
    for item in items {
        if !recipe_exists(conn, &item.source, &item.id).await? {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: "no such recipe".into(),
            });
            continue;
        }
        if let Err(reason) = step::validate(&item.steps) {
            report.rejected.push(Rejection {
                source: item.source,
                id: item.id,
                reason: format!("invalid step graph: {reason}"),
            });
            continue;
        }
        // Only count and re-derive a reading that actually landed: the run-id guard
        // no-ops a write an equal-or-newer run already superseded.
        let wrote = store(conn, &item.source, &item.id, &item.steps, model, store_run).await?;
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

    // The derive run is allocated *here*, after storage — see the doc comment.
    if !accepted.is_empty() {
        let derive_run = runs::begin(conn, "derive").await?;
        report.derived = derive::derive_recipes(conn, &accepted, derive_run)
            .await?
            .derived;
        runs::finish(conn, derive_run, runs::COMPLETED).await?;
    }
    Ok(report)
}

/// Whether a recipe exists — a step submission for an unknown recipe is dropped.
async fn recipe_exists(conn: &Connection, source: &str, id: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM recipes WHERE source = ?1 AND id = ?2",
            libsql::params![source.to_owned(), id.to_owned()],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

// --- Storage + the derive-time join. -------------------------------------------

/// Load every recipe's step reading into a map so [`crate::derive`] can reattach in
/// memory — one query, not a lookup per recipe.
pub async fn load(conn: &Connection) -> anyhow::Result<HashMap<RecipeKey, Vec<StructuredStep>>> {
    let mut rows = conn
        .query("SELECT source, id, structured FROM step_structures", ())
        .await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let structured: String = row.get(2)?;
        // A row that no longer deserializes (a shape change) is skipped, not fatal.
        if let Ok(steps) = serde_json::from_str::<Vec<StructuredStep>>(&structured) {
            map.insert((source, id), steps);
        }
    }
    Ok(map)
}

/// Reattach a recipe's step reading onto `recipe.steps` in place — the join `derive`
/// performs, offline. Unlike ingredient readings, steps align to nothing external
/// (the model segments the method itself), so a stored reading replaces the field
/// wholesale. A recipe with no row keeps `steps: []` — the raw `instructions` stays
/// the source of truth.
pub fn attach(
    steps_by_recipe: &HashMap<RecipeKey, Vec<StructuredStep>>,
    source: &str,
    id: &str,
    steps: &mut Vec<StructuredStep>,
) {
    if let Some(read) = steps_by_recipe.get(&(source.to_owned(), id.to_owned())) {
        *steps = read.clone();
    }
}

/// Write one recipe's step reading, keyed by `(source, id)`, stamped with the model
/// and the run. The `run_id` guard stops a stale or partial run clobbering a newer
/// reading; a deliberate re-read overwrites under a fresh (higher) run id. Returns
/// whether a row was actually written — the guard makes the upsert a no-op when an
/// equal-or-newer run holds the row, and that must not count as stored.
async fn store(
    conn: &Connection,
    source: &str,
    id: &str,
    steps: &[StructuredStep],
    model: &str,
    run_id: i64,
) -> anyhow::Result<bool> {
    let structured = serde_json::to_string(steps)?;
    let affected = conn
        .execute(
            "INSERT INTO step_structures (source, id, structured, model, run_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source, id) DO UPDATE SET
                structured = excluded.structured,
                model      = excluded.model,
                created_at = unixepoch(),
                run_id     = excluded.run_id
             WHERE excluded.run_id >= step_structures.run_id",
            libsql::params![source, id, structured, model, run_id],
        )
        .await?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::StepKind;

    fn ing(name: &str, measure: Option<&str>) -> Ingredient {
        Ingredient {
            name: name.into(),
            measure: measure.map(str::to_owned),
            structured: None,
        }
    }

    fn cook_step(id: u32, seconds: Option<u32>, after: &[u32]) -> StructuredStep {
        StructuredStep {
            id,
            text: format!("step {id}"),
            kind: StepKind::Cook,
            seconds,
            after: after.to_vec(),
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

    async fn insert_recipe(
        conn: &Connection,
        id: &str,
        instructions: &str,
        ingredients: &[Ingredient],
    ) {
        let json = serde_json::to_string(ingredients).unwrap();
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', ?1, 'T', ?2, ?3)",
            libsql::params![id, json, instructions],
        )
        .await
        .unwrap();
    }

    async fn insert_raw(conn: &Connection, id: &str, raw: &str) {
        conn.execute(
            "INSERT INTO raw_imports (source, id, raw, source_url) VALUES ('themealdb', ?1, ?2, ?3)",
            libsql::params![
                id,
                raw,
                format!("https://www.themealdb.com/api/json/v1/1/lookup.php?i={id}")
            ],
        )
        .await
        .unwrap();
    }

    /// Read `recipes.steps` back off the JSON the derive wrote.
    async fn read_recipe_steps(conn: &Connection, id: &str) -> Vec<StructuredStep> {
        let mut rows = conn
            .query(
                "SELECT steps FROM recipes WHERE id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let json: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        serde_json::from_str(&json).unwrap_or_default()
    }

    async fn last_run_id(conn: &Connection, kind: &str) -> i64 {
        let mut rows = conn
            .query(
                "SELECT MAX(id) FROM runs WHERE kind = ?1",
                libsql::params![kind],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    }

    /// `pending` lists exactly the recipes with a method but no step reading, carrying
    /// the instructions and the ingredients; an already-read recipe drops out.
    #[tokio::test]
    async fn pending_lists_unread_methods_with_context() {
        let conn = conn().await;
        insert_recipe(
            &conn,
            "1",
            "Chop. Fry. Simmer.",
            &[ing("Onion", Some("1 sliced"))],
        )
        .await;
        insert_recipe(&conn, "2", "Boil.", &[ing("Egg", Some("2"))]).await;
        store(&conn, "themealdb", "2", &[cook_step(0, None, &[])], "m", 1)
            .await
            .unwrap();

        let p = pending(&conn, 25).await.unwrap();
        assert_eq!(p.len(), 1, "only the unread recipe is pending");
        assert_eq!(p[0].id, "1");
        assert_eq!(p[0].instructions, "Chop. Fry. Simmer.");
        assert_eq!(p[0].ingredients.len(), 1);
        assert_eq!(p[0].ingredients[0].name, "Onion");
    }

    /// A recipe with a blank method never earns a reading, so returning it would loop
    /// the worker forever — it must not appear in `pending`.
    #[tokio::test]
    async fn pending_excludes_blank_methods() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "   ", &[ing("x", None)]).await;
        assert!(
            pending(&conn, 25).await.unwrap().is_empty(),
            "a method-less recipe must not be offered"
        );
    }

    /// `submit` stores a valid graph, rejects a malformed one, and rejects an unknown
    /// recipe — never storing either.
    #[tokio::test]
    async fn submit_stores_valid_and_rejects_the_rest() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Chop then fry.", &[ing("Onion", Some("1"))]).await;

        let items = vec![
            // A well-formed DAG → accepted.
            SubmittedSteps {
                source: "themealdb".into(),
                id: "1".into(),
                steps: vec![cook_step(0, None, &[]), cook_step(1, Some(120), &[0])],
            },
            // A forward dependency → invalid graph → rejected.
            SubmittedSteps {
                source: "themealdb".into(),
                id: "1".into(),
                steps: vec![cook_step(0, None, &[1]), cook_step(1, None, &[])],
            },
            // No such recipe → rejected.
            SubmittedSteps {
                source: "themealdb".into(),
                id: "9".into(),
                steps: vec![cook_step(0, None, &[])],
            },
        ];

        let report = submit(&conn, items, "spy-model").await.unwrap();
        assert_eq!(report.accepted, 1);
        assert_eq!(
            report.rejected.len(),
            2,
            "malformed and unknown are both dropped"
        );

        let loaded = load(&conn).await.unwrap();
        let s1 = loaded.get(&("themealdb".into(), "1".into())).unwrap();
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[1].seconds, Some(120));
    }

    /// Each stored row records which model produced it — provenance for a drifting
    /// source.
    #[tokio::test]
    async fn submit_records_the_model_provenance() {
        let conn = conn().await;
        insert_recipe(&conn, "1", "Go.", &[ing("x", None)]).await;
        submit(
            &conn,
            vec![SubmittedSteps {
                source: "themealdb".into(),
                id: "1".into(),
                steps: vec![cook_step(0, None, &[])],
            }],
            "claude-opus-4-8",
        )
        .await
        .unwrap();

        let mut rows = conn
            .query("SELECT model FROM step_structures WHERE id = '1'", ())
            .await
            .unwrap();
        let model: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(model, "claude-opus-4-8");
    }

    /// A push re-derives under a run allocated after storage, and the reading attaches
    /// onto `recipes.steps` despite `recipes` having been derived by a prior ingest.
    #[tokio::test]
    async fn submit_re_derives_and_attaches_steps() {
        let conn = conn().await;
        insert_raw(
            &conn,
            "1",
            r#"{"meals":[{"idMeal":"1","strMeal":"T","strInstructions":"Chop then fry.","strIngredient1":"Onion","strMeasure1":"1"}]}"#,
        )
        .await;
        let ingest_run = runs::begin(&conn, "ingest").await.unwrap();
        derive::derive(&conn, None, ingest_run).await.unwrap();

        submit(
            &conn,
            vec![SubmittedSteps {
                source: "themealdb".into(),
                id: "1".into(),
                steps: vec![cook_step(0, None, &[]), cook_step(1, Some(60), &[0])],
            }],
            "m",
        )
        .await
        .unwrap();

        let steps = read_recipe_steps(&conn, "1").await;
        assert_eq!(steps.len(), 2, "the reading attached onto recipes.steps");
        assert_eq!(steps[1].seconds, Some(60));

        let store = last_run_id(&conn, "enrich_steps").await;
        let der = last_run_id(&conn, "derive").await;
        assert!(
            der > store && store > ingest_run,
            "derive {der} > store {store} > ingest {ingest_run}"
        );
    }

    /// The run-id guard on the writer: a stale run cannot clobber a newer reading; a
    /// higher run still can.
    #[tokio::test]
    async fn a_stale_run_cannot_clobber_a_reading() {
        let conn = conn().await;
        let first = |loaded: &HashMap<RecipeKey, Vec<StructuredStep>>| {
            loaded
                .get(&("themealdb".to_string(), "1".to_string()))
                .unwrap()[0]
                .text
                .clone()
        };

        let mut s5 = cook_step(0, None, &[]);
        s5.text = "run5".into();
        assert!(store(&conn, "themealdb", "1", &[s5], "m", 5).await.unwrap());

        let mut s3 = cook_step(0, None, &[]);
        s3.text = "run3".into();
        assert!(
            !store(&conn, "themealdb", "1", &[s3], "m", 3).await.unwrap(),
            "a stale write is a no-op"
        );
        assert_eq!(first(&load(&conn).await.unwrap()), "run5");

        let mut s9 = cook_step(0, None, &[]);
        s9.text = "run9".into();
        assert!(store(&conn, "themealdb", "1", &[s9], "m", 9).await.unwrap());
        assert_eq!(first(&load(&conn).await.unwrap()), "run9");
    }

    /// `attach` sets a recipe's steps from the stored reading; a recipe with no row
    /// keeps its steps empty.
    #[test]
    fn attach_sets_steps_and_leaves_the_rest() {
        let mut readings = HashMap::new();
        readings.insert(
            ("themealdb".to_string(), "1".to_string()),
            vec![cook_step(0, None, &[]), cook_step(1, Some(30), &[0])],
        );

        let mut s1 = Vec::new();
        attach(&readings, "themealdb", "1", &mut s1);
        assert_eq!(s1.len(), 2);

        let mut s9 = Vec::new();
        attach(&readings, "themealdb", "9", &mut s9);
        assert!(s9.is_empty(), "no row → steps stay empty");
    }
}
