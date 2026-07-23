//! The corpus store, decoupled by writer.
//!
//! `raw_imports` is what the source actually said; `recipes` is the **derived**
//! view search and browse read, rebuilt from raw by [`crate::derive`] when
//! normalization improves — without re-fetching, which is not reliably possible.
//!
//! Each table has ONE writer: [`store_raw`] persists the fetched payload (called
//! by [`crate::sync`]), and [`upsert`] writes the derived recipe (called solely by
//! [`crate::derive`]). They are no longer coupled into one "store both" call, so
//! there is nothing to tear on a partial write, and "a recipe never exists without
//! its payload" holds by construction — a recipe can only come from deriving a raw
//! import. Both carry a `run_id` and guard on it, so concurrent or stale runs
//! cannot clobber each other (#11 write-path hardening).
//!
//! The backend holds the Turso *write* token; the browser only ever gets a
//! read-only one and reads Turso directly.

use libsql::Connection;
use recipe_core::Recipe;

/// Persist one fetched payload into `raw_imports`, keyed by `(source, id)` — one
/// row per recipe, however many responses mentioned it.
///
/// The ONLY write `sync` makes: `recipes` is derived and written solely by
/// [`upsert`] from [`crate::derive`], so there is no coupled two-halves write to
/// tear on a crash. Raw is not an archive of everything fetched: a category
/// listing is a taxonomy and a browse of partials never reaches the corpus, so
/// neither is stored.
///
/// `run_id` stamps the writing run; the guard `WHERE excluded.run_id >=
/// raw_imports.run_id` lets only an equal-or-newer run overwrite, so a stale or
/// partial older run cannot clobber a newer fetch (`>=`, not `>`, so a run that
/// re-writes its own row — the same recipe named by two responses in one sync —
/// still applies; cross-run is always strict because ids are unique).
pub(crate) async fn store_raw(
    conn: &Connection,
    item: &recipe_core::adapters::Ingested,
    content_type: Option<&str>,
    run_id: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO raw_imports (source, id, raw, content_type, source_url, run_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(source, id) DO UPDATE SET
            raw          = excluded.raw,
            content_type = excluded.content_type,
            source_url   = excluded.source_url,
            fetched_at   = unixepoch(),
            run_id       = excluded.run_id
         WHERE excluded.run_id >= raw_imports.run_id",
        libsql::params![
            item.recipe.source.clone(),
            item.recipe.id.clone(),
            item.raw.clone(),
            content_type.map(str::to_owned),
            item.fetched_from.clone(),
            run_id,
        ],
    )
    .await?;
    Ok(())
}

/// Upsert a recipe keyed by `(source, id)`. `tags` and `ingredients` are stored
/// as JSON; `fetched_at` is refreshed on update.
///
/// **Merge non-empty**: an empty incoming field never overwrites a populated
/// stored one. Sources hand us the same recipe at different completeness — a
/// TheMealDB category browse (`filter.php`) returns header fields only, with no
/// ingredients or instructions — so overwriting column-for-column would let a
/// listing silently blank a full record. An absent field means "this view
/// didn't carry it", not "this recipe has none". `title` is exempt: the handler
/// rejects an empty one, so it is always meaningful.
///
/// The sole writer of `recipes`. `run_id` stamps the deriving run; the guard
/// `WHERE excluded.run_id >= recipes.run_id` lets only an equal-or-newer run
/// overwrite, so a stale or partial older run cannot clobber a newer derive
/// (`>=` so the same run re-deriving its own row applies; cross-run is strict).
/// It composes with the merge-non-empty SET: newer run wins the row, then the
/// per-field merge still protects populated fields from an incoming partial.
pub(crate) async fn upsert(conn: &Connection, recipe: &Recipe, run_id: i64) -> anyhow::Result<()> {
    let tags = serde_json::to_string(&recipe.tags)?;
    let ingredients = serde_json::to_string(&recipe.ingredients)?;
    let steps = serde_json::to_string(&recipe.steps)?;
    conn.execute(
        "INSERT INTO recipes
            (source, id, title, image, category, area, tags, ingredients, instructions, source_url, video_url, steps, run_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(source, id) DO UPDATE SET
            title        = excluded.title,
            image        = COALESCE(NULLIF(excluded.image, ''), recipes.image),
            category     = COALESCE(NULLIF(excluded.category, ''), recipes.category),
            area         = COALESCE(NULLIF(excluded.area, ''), recipes.area),
            tags         = CASE WHEN json_array_length(excluded.tags) > 0
                                THEN excluded.tags ELSE recipes.tags END,
            ingredients  = CASE WHEN json_array_length(excluded.ingredients) > 0
                                THEN excluded.ingredients ELSE recipes.ingredients END,
            instructions = CASE WHEN trim(excluded.instructions) <> ''
                                THEN excluded.instructions ELSE recipes.instructions END,
            steps        = CASE WHEN json_array_length(excluded.steps) > 0
                                THEN excluded.steps ELSE recipes.steps END,
            source_url   = COALESCE(NULLIF(excluded.source_url, ''), recipes.source_url),
            video_url    = COALESCE(NULLIF(excluded.video_url, ''), recipes.video_url),
            fetched_at   = unixepoch(),
            run_id       = excluded.run_id
         WHERE excluded.run_id >= recipes.run_id",
        libsql::params![
            recipe.source.clone(),
            recipe.id.clone(),
            recipe.title.clone(),
            recipe.image.clone(),
            recipe.category.clone(),
            recipe.area.clone(),
            tags,
            ingredients,
            recipe.instructions.clone(),
            recipe.source_url.clone(),
            recipe.video_url.clone(),
            steps,
            run_id,
        ],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::Ingredient;

    fn sample() -> Recipe {
        Recipe {
            id: "1".into(),
            source: "themealdb".into(),
            title: "Soup".into(),
            image: Some("img".into()),
            category: Some("Starter".into()),
            area: None,
            tags: vec!["easy".into()],
            ingredients: vec![Ingredient {
                name: "water".into(),
                measure: Some("1 cup".into()),
                structured: None,
            }],
            instructions: "Boil.".into(),
            steps: Vec::new(),
            equipment: Vec::new(),
            source_url: None,
            video_url: None,
        }
    }

    /// A category browse (`filter.php`) shaped record: header fields only.
    fn partial() -> Recipe {
        Recipe {
            id: "1".into(),
            source: "themealdb".into(),
            title: "Soup".into(),
            image: Some("img".into()),
            category: Some("Starter".into()),
            area: None,
            tags: vec![],
            ingredients: vec![],
            instructions: String::new(),
            steps: Vec::new(),
            equipment: Vec::new(),
            source_url: None,
            video_url: None,
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

    async fn read(conn: &Connection) -> (String, String, String, Option<String>) {
        let mut rows = conn
            .query(
                "SELECT instructions, ingredients, tags, area FROM recipes
                 WHERE source = ?1 AND id = ?2",
                libsql::params!["themealdb", "1"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (
            row.get::<String>(0).unwrap(),
            row.get::<String>(1).unwrap(),
            row.get::<String>(2).unwrap(),
            row.get::<Option<String>>(3).unwrap(),
        )
    }

    /// The bug this guards: browsing a category yields partials, and a
    /// column-for-column upsert would blank a stored full recipe's detail.
    #[tokio::test]
    async fn partial_does_not_clobber_a_full_record() {
        let conn = conn().await;

        let mut full = sample();
        full.area = Some("Italian".into());
        upsert(&conn, &full, 1).await.unwrap();

        upsert(&conn, &partial(), 1).await.unwrap();

        let (instructions, ingredients, tags, area) = read(&conn).await;
        assert_eq!(instructions, "Boil.", "instructions must survive a partial");
        assert!(
            ingredients.contains("water"),
            "ingredients must survive a partial, got {ingredients}"
        );
        assert_eq!(tags, r#"["easy"]"#, "tags must survive a partial");
        assert_eq!(
            area.as_deref(),
            Some("Italian"),
            "area must survive a partial"
        );
    }

    /// The other direction still has to work: a full record fills in a partial.
    #[tokio::test]
    async fn full_upgrades_a_partial_record() {
        let conn = conn().await;

        upsert(&conn, &partial(), 1).await.unwrap();
        let (instructions, ingredients, ..) = read(&conn).await;
        assert_eq!(instructions, "");
        assert_eq!(ingredients, "[]");

        upsert(&conn, &sample(), 1).await.unwrap();

        let (instructions, ingredients, tags, _) = read(&conn).await;
        assert_eq!(instructions, "Boil.");
        assert!(ingredients.contains("water"));
        assert_eq!(tags, r#"["easy"]"#);
    }

    /// Merging must not freeze a field: a non-empty value still overwrites.
    #[tokio::test]
    async fn non_empty_still_overwrites() {
        let conn = conn().await;
        upsert(&conn, &sample(), 1).await.unwrap();

        let mut revised = sample();
        revised.instructions = "Simmer gently.".into();
        revised.area = Some("French".into());
        upsert(&conn, &revised, 1).await.unwrap();

        let (instructions, _, _, area) = read(&conn).await;
        assert_eq!(instructions, "Simmer gently.");
        assert_eq!(area.as_deref(), Some("French"));
    }

    /// The run-id guard: a lower (older or partial) run cannot overwrite a row a
    /// newer run already wrote; a higher run still can. This is what stops a stale
    /// or concurrent run clobbering another (#11 write-path hardening).
    async fn title_and_run(conn: &Connection) -> (String, i64) {
        let mut rows = conn
            .query(
                "SELECT title, run_id FROM recipes WHERE source = 'themealdb' AND id = '1'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (row.get::<String>(0).unwrap(), row.get::<i64>(1).unwrap())
    }

    #[tokio::test]
    async fn a_stale_run_cannot_clobber_a_newer_one() {
        let conn = conn().await;
        let mut r = sample();

        r.title = "run 5".into();
        upsert(&conn, &r, 5).await.unwrap();
        assert_eq!(title_and_run(&conn).await, ("run 5".into(), 5));

        // An older run writing late must be a no-op — not a clobber.
        r.title = "run 3 (stale)".into();
        upsert(&conn, &r, 3).await.unwrap();
        assert_eq!(
            title_and_run(&conn).await,
            ("run 5".into(), 5),
            "an older run must not overwrite a newer one"
        );

        // A newer run still wins.
        r.title = "run 9".into();
        upsert(&conn, &r, 9).await.unwrap();
        assert_eq!(title_and_run(&conn).await, ("run 9".into(), 9));
    }

    async fn count(conn: &Connection, table: &str) -> i64 {
        let mut rows = conn
            .query(&format!("SELECT count(*) FROM {table}"), ())
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    }

    fn ingested(raw: &str) -> recipe_core::adapters::Ingested {
        recipe_core::adapters::Ingested {
            recipe: sample(),
            raw: raw.into(),
            fetched_from: "https://themealdb.test/1".into(),
        }
    }

    /// Decoupled: `store_raw` writes ONLY `raw_imports`, never `recipes` — recipes
    /// is derive's alone, so there is no coupled two-halves write to tear.
    #[tokio::test]
    async fn store_raw_writes_only_raw_not_recipes() {
        let conn = conn().await;
        store_raw(
            &conn,
            &ingested(r#"{"meals":[]}"#),
            Some("application/json"),
            1,
        )
        .await
        .unwrap();
        assert_eq!(count(&conn, "raw_imports").await, 1, "raw is written");
        assert_eq!(
            count(&conn, "recipes").await,
            0,
            "recipes is not — that's derive's job"
        );
    }

    /// The guard on the source-of-truth writer too: a stale run cannot clobber a
    /// newer fetch of the same raw.
    #[tokio::test]
    async fn store_raw_stale_run_cannot_clobber() {
        let conn = conn().await;
        store_raw(&conn, &ingested("newer"), None, 5).await.unwrap();
        store_raw(&conn, &ingested("stale"), None, 3).await.unwrap();

        let mut rows = conn
            .query("SELECT raw, run_id FROM raw_imports WHERE id = '1'", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(
            row.get::<String>(0).unwrap(),
            "newer",
            "an older run must not clobber a newer fetch"
        );
        assert_eq!(row.get::<i64>(1).unwrap(), 5);
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates_on_conflict() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();

        let mut recipe = sample();
        upsert(&conn, &recipe, 1).await.unwrap();

        let mut rows = conn
            .query(
                "SELECT title, tags FROM recipes WHERE source = ?1 AND id = ?2",
                libsql::params!["themealdb", "1"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "Soup");
        assert_eq!(row.get::<String>(1).unwrap(), r#"["easy"]"#);

        // Same (source, id) updates in place — no duplicate row.
        recipe.title = "Better Soup".into();
        upsert(&conn, &recipe, 1).await.unwrap();

        let mut rows = conn
            .query("SELECT count(*), max(title) FROM recipes", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<i64>(0).unwrap(), 1);
        assert_eq!(row.get::<String>(1).unwrap(), "Better Soup");
    }
}
