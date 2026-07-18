//! Derive the `recipes` view from `raw_imports`.
//!
//! `recipes` is a **derived** table: every row is a normalization of a payload
//! in `raw_imports`. Deriving is therefore not a repair mode — it is the same
//! step ingest performs, replayed over what we already hold. When normalization
//! improves (an adapter fix, a new extracted field, #11's enrichment), this
//! reaches rows imported before the fix existed, **with zero upstream calls**.
//!
//! That matters because re-fetching is not a reliable recovery plan: sources 502
//! scrapers (Serious Eats already does), disappear, and paywall.

use libsql::Connection;
use recipe_core::adapters;

use crate::enrich;
use crate::recipes::upsert;

/// What a derive run did.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    /// Raw payloads read.
    pub read: usize,
    /// Recipes upserted.
    pub derived: usize,
    /// Payloads whose adapter no longer exists, or that no longer normalize to
    /// anything. Kept, never deleted — the payload is the source of truth, and a
    /// normalizer that cannot read it today is our bug to fix, not its fault.
    pub skipped: usize,
}

/// Rebuild `recipes` from every stored payload. Optionally limited to one
/// source, so fixing one adapter need not replay the whole corpus.
pub async fn derive(
    conn: &Connection,
    source: Option<&str>,
    run_id: i64,
) -> anyhow::Result<Report> {
    let mut report = Report::default();

    // The structured readings (#11), loaded once so reattaching a recipe's
    // readings is an in-memory lookup rather than a query per recipe. Empty when
    // nothing has been enriched — recipes then just keep `structured: None`, which
    // is why deriving works with or without enrichment having run.
    let readings = enrich::load(conn).await?;

    let mut rows = match source {
        Some(source) => {
            conn.query(
                "SELECT source, id, raw, source_url FROM raw_imports WHERE source = ?1",
                libsql::params![source],
            )
            .await?
        }
        None => {
            conn.query("SELECT source, id, raw, source_url FROM raw_imports", ())
                .await?
        }
    };

    while let Some(row) = rows.next().await? {
        report.read += 1;
        let source: String = row.get(0)?;
        let id: String = row.get(1)?;
        let raw: String = row.get(2)?;
        let source_url: Option<String> = row.get(3)?;

        let Some(adapter) = adapters::adapter_by_id(&source) else {
            report.skipped += 1;
            continue;
        };

        // A payload is a document for its own adapter, so deriving runs the
        // ingest path rather than a parallel one. schema.org reads a recipe's id
        // and source_url off the URL, so pass the URL it was fetched at.
        let url = source_url.unwrap_or_else(|| format!("https://{source}/{id}"));
        let Ok(parsed) = url::Url::parse(&url) else {
            report.skipped += 1;
            continue;
        };

        let ingested = (adapter.normalize)(&parsed, &raw);
        if ingested.is_empty() {
            report.skipped += 1;
            continue;
        }
        for mut item in ingested {
            // Reattach the enrichment half. Normalization produces `structured:
            // None`; `derive` is the join that fills it from the recipe's readings.
            enrich::attach(
                &readings,
                &item.recipe.source,
                &item.recipe.id,
                &mut item.recipe.ingredients,
            );
            upsert(conn, &item.recipe, run_id).await?;
            report.derived += 1;
        }
    }

    Ok(report)
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

    /// The acceptance: `recipes` is rebuilt entirely from raw, no network.
    #[tokio::test]
    async fn derives_recipes_from_raw_with_no_network() {
        let conn = conn().await;
        insert_raw(
            &conn,
            "1",
            r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1 slice"}]}"#,
        )
        .await;

        // recipes starts empty — nothing has been derived yet.
        let mut rows = conn
            .query("SELECT count(*) FROM recipes", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0
        );

        let report = derive(&conn, None, 1).await.unwrap();
        assert_eq!(
            report,
            Report {
                read: 1,
                derived: 1,
                skipped: 0
            }
        );

        let mut rows = conn
            .query("SELECT title, instructions FROM recipes WHERE id = '1'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "Toast");
        assert_eq!(row.get::<String>(1).unwrap(), "Toast it.");
    }

    /// Deriving is idempotent: it is the ingest step replayed, not a migration.
    #[tokio::test]
    async fn deriving_twice_is_stable() {
        let conn = conn().await;
        insert_raw(
            &conn,
            "1",
            r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1"}]}"#,
        )
        .await;

        derive(&conn, None, 1).await.unwrap();
        derive(&conn, None, 1).await.unwrap();

        let mut rows = conn
            .query("SELECT count(*) FROM recipes", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1
        );
    }

    /// A payload whose adapter is gone is skipped, never deleted — raw is the
    /// source of truth and outlives whatever could read it.
    #[tokio::test]
    async fn unknown_source_is_skipped_not_dropped() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO raw_imports (source, id, raw) VALUES ('retired-source', '9', '{}')",
            (),
        )
        .await
        .unwrap();

        let report = derive(&conn, None, 1).await.unwrap();
        assert_eq!(report.skipped, 1);
        assert_eq!(report.derived, 0);

        let mut rows = conn
            .query("SELECT count(*) FROM raw_imports", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1,
            "raw must survive a normalizer that cannot read it"
        );
    }

    async fn read_ingredients(conn: &Connection, id: &str) -> Vec<recipe_core::Ingredient> {
        let mut rows = conn
            .query(
                "SELECT ingredients FROM recipes WHERE id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let json: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    /// Deriving reattaches a recipe's stored readings onto its ingredients (#11) —
    /// the offline join, per recipe. A recipe with a matching readings array gets
    /// them all; a recipe with no row stays `None`. This is acceptance #1: after
    /// derive, an enriched recipe carries the structured list, raw preserved.
    #[tokio::test]
    async fn derive_reattaches_stored_readings_per_recipe() {
        let conn = conn().await;
        // Recipe 1: two ingredients, with a matching stored readings array.
        insert_raw(
            &conn,
            "1",
            r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1 slice","strIngredient2":"Butter","strMeasure2":"1 tbsp"}]}"#,
        )
        .await;
        conn.execute(
            "INSERT INTO ingredient_structures (source, id, structured) VALUES ('themealdb', '1', ?1)",
            libsql::params![
                r#"[{"item":"bread","amount":null,"preparation":"toasted","note":null},{"item":"butter","amount":null,"preparation":null,"note":null}]"#
            ],
        )
        .await
        .unwrap();
        // Recipe 2: has ingredients but no stored readings.
        insert_raw(
            &conn,
            "2",
            r#"{"meals":[{"idMeal":"2","strMeal":"Water","strInstructions":"Pour.","strIngredient1":"Water","strMeasure1":"1 cup"}]}"#,
        )
        .await;

        derive(&conn, None, 1).await.unwrap();

        let one = read_ingredients(&conn, "1").await;
        assert_eq!(one.len(), 2);
        assert_eq!(one[0].measure.as_deref(), Some("1 slice"), "raw preserved");
        assert_eq!(one[0].structured.as_ref().unwrap().item, "bread");
        assert_eq!(
            one[0].structured.as_ref().unwrap().preparation.as_deref(),
            Some("toasted")
        );
        assert_eq!(one[1].structured.as_ref().unwrap().item, "butter");

        let two = read_ingredients(&conn, "2").await;
        assert_eq!(
            two[0].structured, None,
            "a recipe with no stored readings stays None"
        );
    }

    /// A fix to normalization reaches rows imported before it — the whole point.
    #[tokio::test]
    async fn derive_repairs_a_stale_derived_row() {
        let conn = conn().await;
        insert_raw(
            &conn,
            "1",
            r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1"}]}"#,
        )
        .await;
        // Simulate a row derived by an older, worse normalizer.
        conn.execute(
            "INSERT INTO recipes (source, id, title, instructions) VALUES ('themealdb','1','WRONG','')",
            (),
        )
        .await
        .unwrap();

        derive(&conn, None, 1).await.unwrap();

        let mut rows = conn
            .query("SELECT title, instructions FROM recipes WHERE id = '1'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "Toast");
        assert_eq!(row.get::<String>(1).unwrap(), "Toast it.");
    }
}
