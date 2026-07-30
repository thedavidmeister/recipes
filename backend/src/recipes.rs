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
    // The total-time estimate (#79) is pure arithmetic over the very steps being
    // written — the critical path through the DAG — computed here in recipe-core so the
    // stored column always matches the `steps` beside it. `None` (→ NULL) when there is
    // no timing signal (un-read, or nothing timed): absence, not a wrong `0`.
    let total_seconds = recipe.total_seconds().map(i64::from);
    // Whether that estimate is complete or only a floor (#158/#84) — read off the very
    // same steps, right here, so the two can never describe different graphs. It
    // decides the mark #84 renders: `~25 min` when every step counted, `25 min+` when
    // one contributed 0 and the total can therefore only be too low.
    let fully_timed = i64::from(recipe.fully_timed());
    // The equipment reading (#81) `derive` has just reattached. Migration 0014 added
    // the column and `derive` fills the field, but this — the sole writer of `recipes`
    // — never carried it, so every derived row kept the `'[]'` default and the reading
    // was recomputed and thrown away on every run. Measured against production before
    // fixing it: 790/790 recipes had a reading in `equipment_structures`, and 790/790
    // rows of `recipes` read `'[]'`. Matching a kitchen against a recipe (#82, #83)
    // reads this column, so it has to actually hold the reading.
    let equipment = serde_json::to_string(&recipe.equipment)?;
    // The nutrition reading (#162) `derive` has just reattached, and the total computed
    // from it right here — off the very field being written, exactly as `total_seconds`
    // is computed off the `steps` beside it. All four columns are named in the INSERT
    // *and* in the ON CONFLICT SET below, because the #161 lesson is that a derived
    // column the sole writer of this table never lists is recomputed and thrown away on
    // every run, invisibly, for as long as nothing needs it.
    let nutrition = serde_json::to_string(&recipe.nutrition)?;
    let energy = recipe.energy();
    // Whole kcal — the estimate is nowhere near accurate to a calorie, and storing a
    // float would invite a display that renders one. `None` (→ NULL) when nothing could
    // be counted: absence, not a `0` that would read as a dish with no calories.
    let kcal = energy.map(|e| e.kcal.round() as i64);
    // Whether that total counted every line that stated a number, or is only a floor.
    // Read off the same reading in the same place, so the two can never describe
    // different data — the `fully_timed` precedent exactly.
    let kcal_complete = i64::from(energy.is_some_and(|e| e.complete()));
    let servings = recipe.servings.map(i64::from);
    // The meal-time reading (#191) `derive` has just reattached: the set of sittings the
    // dish suits. Named in the INSERT *and* in the ON CONFLICT SET below — the #161
    // lesson, which is that a derived column the sole writer of this table never lists is
    // recomputed and thrown away on every run, invisibly, for as long as nothing needs
    // it. The walk reads this column on every channelled request, so it has to actually
    // hold the reading.
    let sittings = serde_json::to_string(&recipe.sittings)?;
    conn.execute(
        "INSERT INTO recipes
            (source, id, title, image, category, area, tags, ingredients, instructions, source_url, video_url, steps, total_seconds, fully_timed, equipment, nutrition, servings, kcal, kcal_complete, sittings, run_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
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
            -- Move in lockstep with `steps`: the estimate is a function of the steps,
            -- so it must follow whichever `steps` wins the merge. A partial browse
            -- (empty steps) keeps both the stored steps and their stored estimate.
            total_seconds = CASE WHEN json_array_length(excluded.steps) > 0
                                THEN excluded.total_seconds ELSE recipes.total_seconds END,
            -- Same lockstep, same reason: it qualifies that estimate, so the two must
            -- always describe the same `steps`. Listing it here is the whole job — a
            -- derived column the sole writer never named is exactly how `equipment`
            -- sat at its `'[]'` default for months across all 790 rows (#161).
            fully_timed  = CASE WHEN json_array_length(excluded.steps) > 0
                                THEN excluded.fully_timed ELSE recipes.fully_timed END,
            -- Merge-non-empty like `steps`, and for the sharper reason that `[]` is
            -- not a reading at all here (#81 refuses an empty one on the way in): an
            -- incoming `[]` means unread, so it must never blank a reading we hold.
            equipment    = CASE WHEN json_array_length(excluded.equipment) > 0
                                THEN excluded.equipment ELSE recipes.equipment END,
            -- Merge-non-empty like `equipment`, same reason: `[]` is not a reading,
            -- it is the absence of one, so a partial browse must not blank a stored
            -- reading with it.
            nutrition    = CASE WHEN json_array_length(excluded.nutrition) > 0
                                THEN excluded.nutrition ELSE recipes.nutrition END,
            -- The three that follow from that reading move in lockstep with it, the
            -- way `total_seconds`/`fully_timed` move with `steps`. Gating them on the
            -- incoming `nutrition` rather than on their own NULL-ness is what keeps
            -- the four consistent: whichever reading wins the row, its total,
            -- completeness and serving count win with it.
            servings     = CASE WHEN json_array_length(excluded.nutrition) > 0
                                THEN excluded.servings ELSE recipes.servings END,
            kcal         = CASE WHEN json_array_length(excluded.nutrition) > 0
                                THEN excluded.kcal ELSE recipes.kcal END,
            kcal_complete = CASE WHEN json_array_length(excluded.nutrition) > 0
                                THEN excluded.kcal_complete ELSE recipes.kcal_complete END,
            -- Merge-non-empty like `equipment`, and for the same sharp reason: `[]` is
            -- not a reading here, it is the absence of one (#191 refuses an empty set on
            -- the way in, because every dish is eaten at some time), so an incoming empty
            -- must never blank a reading we hold. A category browse would otherwise
            -- silently un-read the corpus and quietly widen every plan's deck.
            sittings     = CASE WHEN json_array_length(excluded.sittings) > 0
                                THEN excluded.sittings ELSE recipes.sittings END,
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
            total_seconds,
            fully_timed,
            equipment,
            nutrition,
            servings,
            kcal,
            kcal_complete,
            sittings,
            run_id,
        ],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::{Ingredient, StepKind, StructuredStep};

    fn cook_step(id: u32, seconds: Option<u32>, after: &[u32]) -> StructuredStep {
        StructuredStep {
            id,
            text: format!("step {id}"),
            kind: StepKind::Cook,
            seconds,
            after: after.to_vec(),
        }
    }

    async fn read_total_seconds(conn: &Connection, id: &str) -> Option<i64> {
        let mut rows = conn
            .query(
                "SELECT total_seconds FROM recipes WHERE source = 'themealdb' AND id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<Option<i64>>(0)
            .unwrap()
    }

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
            nutrition: Vec::new(),
            servings: None,
            sittings: Vec::new(),
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
            nutrition: Vec::new(),
            servings: None,
            sittings: Vec::new(),
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

    /// The derived `recipes` view carries the #79 total-time estimate: `upsert`
    /// computes the critical path from the steps it is writing and stores it. A diamond
    /// DAG stores its longest path (390s here: 60 -> 300 -> 30), not the flat sum.
    #[tokio::test]
    async fn upsert_stores_the_critical_path_total() {
        let conn = conn().await;
        let mut r = sample();
        r.steps = vec![
            cook_step(0, Some(60), &[]),
            cook_step(1, Some(120), &[0]),
            cook_step(2, Some(300), &[0]),
            cook_step(3, Some(30), &[1, 2]),
        ];
        upsert(&conn, &r, 1).await.unwrap();
        assert_eq!(read_total_seconds(&conn, "1").await, Some(390));
    }

    /// Degrade-not-die at the column: a recipe whose steps carry no timer stores NULL,
    /// never a wrong `0`. A recipe with no steps at all likewise has no estimate.
    #[tokio::test]
    async fn upsert_stores_null_when_there_is_no_timing_signal() {
        let conn = conn().await;

        let mut untimed = sample();
        untimed.steps = vec![cook_step(0, None, &[]), cook_step(1, None, &[0])];
        upsert(&conn, &untimed, 1).await.unwrap();
        assert_eq!(
            read_total_seconds(&conn, "1").await,
            None,
            "a fully-untimed graph has no estimate"
        );

        // sample() has no steps at all — also no estimate.
        let mut stepless = sample();
        stepless.id = "2".into();
        upsert(&conn, &stepless, 1).await.unwrap();
        assert_eq!(read_total_seconds(&conn, "2").await, None);
    }

    /// The estimate moves in lockstep with `steps`: a partial browse (empty steps)
    /// must not blank a full record's stored steps *or* its stored total — the two
    /// would otherwise desync.
    #[tokio::test]
    async fn a_partial_does_not_clobber_the_stored_total() {
        let conn = conn().await;

        let mut full = sample();
        full.steps = vec![cook_step(0, Some(300), &[]), cook_step(1, Some(60), &[0])];
        upsert(&conn, &full, 1).await.unwrap();
        assert_eq!(read_total_seconds(&conn, "1").await, Some(360));

        // A category browse carries no steps; it must leave the estimate intact.
        upsert(&conn, &partial(), 1).await.unwrap();
        assert_eq!(
            read_total_seconds(&conn, "1").await,
            Some(360),
            "an empty-steps partial must not blank the stored estimate"
        );
    }

    /// Lockstep also when the news is bad: a fresh reading whose steps carry no timers
    /// replaces the stored steps, so it must take the stored estimate down to NULL with
    /// it — keeping the old number would leave it describing steps that are gone.
    #[tokio::test]
    async fn a_fresh_untimed_reading_nulls_the_stored_estimate() {
        let conn = conn().await;

        let mut timed = sample();
        timed.steps = vec![cook_step(0, Some(300), &[]), cook_step(1, Some(60), &[0])];
        upsert(&conn, &timed, 1).await.unwrap();
        assert_eq!(read_total_seconds(&conn, "1").await, Some(360));

        // The recipe is re-read and the new DAG has no timers ("until golden", twice).
        let mut untimed = sample();
        untimed.steps = vec![cook_step(0, None, &[]), cook_step(1, None, &[0])];
        upsert(&conn, &untimed, 2).await.unwrap();
        assert_eq!(
            read_total_seconds(&conn, "1").await,
            None,
            "the estimate follows the steps it summarises, even down to NULL"
        );
    }

    async fn read_equipment(conn: &Connection, id: &str) -> String {
        let mut rows = conn
            .query(
                "SELECT equipment FROM recipes WHERE source = 'themealdb' AND id = ?1",
                libsql::params![id],
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

    fn eq(item: &str) -> recipe_core::equipment::RequiredEquipment {
        recipe_core::equipment::RequiredEquipment { item: item.into() }
    }

    /// The derived view actually carries the equipment reading (#81). It did not: the
    /// column existed and `derive` filled the field, but `upsert` never wrote it, so
    /// all 790 production rows sat at the `'[]'` default while all 790 readings existed
    /// in `equipment_structures`. Everything that matches a kitchen against a recipe
    /// (#82, #83) reads this column, so it is pinned here.
    #[tokio::test]
    async fn upsert_stores_the_equipment_reading() {
        let conn = conn().await;
        let mut r = sample();
        r.equipment = vec![eq("knife"), eq("bowl")];
        upsert(&conn, &r, 1).await.unwrap();
        assert_eq!(
            read_equipment(&conn, "1").await,
            r#"[{"item":"knife"},{"item":"bowl"}]"#
        );
    }

    /// A recipe nobody has read for equipment stores `[]` — the degrade-not-die state,
    /// which reads as "unread", never as "needs nothing" (#81).
    #[tokio::test]
    async fn an_unread_recipe_stores_an_empty_reading() {
        let conn = conn().await;
        upsert(&conn, &sample(), 1).await.unwrap();
        assert_eq!(read_equipment(&conn, "1").await, "[]");
    }

    /// Merge-non-empty for the reading too, and here it matters more than elsewhere:
    /// `[]` is not a reading at all (#81 refuses one), so an incoming empty must never
    /// blank a reading we already hold — a category browse would otherwise silently
    /// un-read the corpus.
    #[tokio::test]
    async fn a_partial_does_not_clobber_the_stored_reading() {
        let conn = conn().await;
        let mut full = sample();
        full.equipment = vec![eq("wok")];
        upsert(&conn, &full, 1).await.unwrap();

        upsert(&conn, &partial(), 1).await.unwrap();
        assert_eq!(
            read_equipment(&conn, "1").await,
            r#"[{"item":"wok"}]"#,
            "an unread partial must not blank the stored reading"
        );
    }

    // --- The nutrition reading (#162) ------------------------------------------

    /// Everything the derived view claims about a recipe's calories, in one read:
    /// the reading, the serving count, the total, and whether that total is complete.
    async fn read_nutrition(
        conn: &Connection,
        id: &str,
    ) -> (String, Option<i64>, Option<i64>, i64) {
        let mut rows = conn
            .query(
                "SELECT nutrition, servings, kcal, kcal_complete FROM recipes
                 WHERE source = 'themealdb' AND id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (
            row.get::<String>(0).unwrap(),
            row.get::<Option<i64>>(1).unwrap(),
            row.get::<Option<i64>>(2).unwrap(),
            row.get::<i64>(3).unwrap(),
        )
    }

    /// A recipe carrying both readings: `grams` of a food at `kcal_per_100g`, with the
    /// ingredient reading that gives the quantity. Built from a mass unit so the
    /// arithmetic here needs nothing from the nutrition reading but the density.
    fn weighed(grams: f64, kcal_per_100g: f64, servings: u32) -> Recipe {
        let mut r = sample();
        r.ingredients = vec![Ingredient {
            name: "chicken".into(),
            measure: Some(format!("{grams} g")),
            structured: Some(recipe_core::StructuredMeasure {
                item: "chicken".into(),
                amount: Some(recipe_core::Amount::Quantified {
                    quantity: recipe_core::Quantity::Exact { value: grams },
                    unit: Some("g".into()),
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        }];
        r.nutrition = vec![recipe_core::FoodEnergy {
            kcal_per_100g,
            grams_per_unit: None,
        }];
        r.servings = Some(servings);
        r
    }

    /// **The #161 round-trip, written with the migration rather than after it.** The
    /// equipment reading was correctly produced and reattached for months while
    /// `upsert` never listed the column, so all 790 rows read `[]` and nobody could
    /// tell. Every column this migration adds is pinned here on the way in: the
    /// reading itself, the serving count, the computed total, and its completeness.
    #[tokio::test]
    async fn upsert_stores_the_nutrition_reading_and_everything_derived_from_it() {
        let conn = conn().await;
        // 500 g at 209 kcal/100 g = 1045 kcal, serving 4.
        upsert(&conn, &weighed(500.0, 209.0, 4), 1).await.unwrap();

        let (nutrition, servings, kcal, complete) = read_nutrition(&conn, "1").await;
        assert_eq!(nutrition, r#"[{"kcal_per_100g":209.0}]"#);
        assert_eq!(servings, Some(4));
        assert_eq!(kcal, Some(1045), "the total is computed, not carried");
        assert_eq!(complete, 1, "every line that stated a number was counted");
    }

    /// A recipe nobody has read stores `[]` and NULLs — degrade-not-die, and never a
    /// `0` kcal, which would render as a dish with no calories.
    #[tokio::test]
    async fn an_unread_recipe_stores_no_calories_rather_than_zero() {
        let conn = conn().await;
        upsert(&conn, &sample(), 1).await.unwrap();
        assert_eq!(
            read_nutrition(&conn, "1").await,
            ("[]".into(), None, None, 0)
        );
    }

    /// A line with a number we could not weigh makes the total a floor, and the column
    /// says so — the `fully_timed` bargain, applied to calories. The `cup` line has no
    /// `grams_per_unit`, so nothing can turn it into grams.
    #[tokio::test]
    async fn a_line_that_cannot_be_weighed_marks_the_total_incomplete() {
        let conn = conn().await;
        let mut r = weighed(500.0, 209.0, 4);
        r.ingredients.push(Ingredient {
            name: "flour".into(),
            measure: Some("1 cup".into()),
            structured: Some(recipe_core::StructuredMeasure {
                item: "flour".into(),
                amount: Some(recipe_core::Amount::Quantified {
                    quantity: recipe_core::Quantity::Exact { value: 1.0 },
                    unit: Some("cup".into()),
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        });
        r.nutrition.push(recipe_core::FoodEnergy {
            kcal_per_100g: 364.0,
            grams_per_unit: None,
        });

        upsert(&conn, &r, 1).await.unwrap();
        let (_, _, kcal, complete) = read_nutrition(&conn, "1").await;
        assert_eq!(kcal, Some(1045), "only the weighable line is summed");
        assert_eq!(complete, 0, "and the total is marked a floor");
    }

    /// The reading and the three columns derived from it move in lockstep, so a
    /// category browse — which carries no reading — cannot blank a stored total while
    /// leaving the reading, or the reverse. Both halves matter: a stored `kcal` with no
    /// `nutrition` beside it is a number nothing can re-derive or audit.
    #[tokio::test]
    async fn a_partial_does_not_clobber_the_stored_calories() {
        let conn = conn().await;
        upsert(&conn, &weighed(500.0, 209.0, 4), 1).await.unwrap();
        upsert(&conn, &partial(), 1).await.unwrap();

        let (nutrition, servings, kcal, complete) = read_nutrition(&conn, "1").await;
        assert_eq!(nutrition, r#"[{"kcal_per_100g":209.0}]"#);
        assert_eq!(servings, Some(4));
        assert_eq!(kcal, Some(1045));
        assert_eq!(complete, 1);
    }

    /// Lockstep when the news is bad, too: a fresh reading replaces the stored one, so
    /// its total replaces the stored total even when the new one is worse. Keeping the
    /// old number would leave it describing a reading that is gone — the same trap
    /// `a_fresh_untimed_reading_nulls_the_stored_estimate` pins for time.
    #[tokio::test]
    async fn a_fresh_reading_replaces_the_stored_total() {
        let conn = conn().await;
        upsert(&conn, &weighed(500.0, 209.0, 4), 1).await.unwrap();
        // A re-read that says the same food is far less dense, and feeds two.
        upsert(&conn, &weighed(500.0, 100.0, 2), 2).await.unwrap();

        let (_, servings, kcal, _) = read_nutrition(&conn, "1").await;
        assert_eq!(kcal, Some(500));
        assert_eq!(servings, Some(2));
    }

    // --- The meal-time reading (#191) ------------------------------------------

    async fn read_sittings(conn: &Connection, id: &str) -> String {
        let mut rows = conn
            .query(
                "SELECT sittings FROM recipes WHERE source = 'themealdb' AND id = ?1",
                libsql::params![id],
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

    /// **The #161 round-trip, written with the migration rather than after it.** The
    /// equipment reading was correctly produced and reattached for months while `upsert`
    /// never listed the column, so all 790 rows read `[]` and nobody could tell. The
    /// walk reads this column on every channelled request, so it has to hold the
    /// reading — and this is the test that says the sole writer names it.
    #[tokio::test]
    async fn upsert_stores_the_meal_time_reading() {
        let conn = conn().await;
        let mut r = sample();
        r.sittings = vec![recipe_core::Sitting::Lunch, recipe_core::Sitting::Dinner];
        upsert(&conn, &r, 1).await.unwrap();
        assert_eq!(read_sittings(&conn, "1").await, r#"["lunch","dinner"]"#);
    }

    /// A recipe nobody has read stores `[]` — the degrade-not-die state, which reads as
    /// "unread" and narrows nobody's deck, never as "eaten at no sitting" (#191 refuses
    /// an empty reading on the way in, so `[]` can only mean unread).
    #[tokio::test]
    async fn an_unread_recipe_stores_no_sittings() {
        let conn = conn().await;
        upsert(&conn, &sample(), 1).await.unwrap();
        assert_eq!(read_sittings(&conn, "1").await, "[]");
    }

    /// Merge-non-empty, and here it matters as much as it does for equipment: `[]` is
    /// the absence of a reading, so an incoming empty must never blank one we hold. A
    /// category browse would otherwise silently un-read the corpus and quietly widen
    /// every plan's deck back to everything.
    #[tokio::test]
    async fn a_partial_does_not_clobber_the_stored_sittings() {
        let conn = conn().await;
        let mut full = sample();
        full.sittings = vec![recipe_core::Sitting::Dinner];
        upsert(&conn, &full, 1).await.unwrap();

        upsert(&conn, &partial(), 1).await.unwrap();
        assert_eq!(
            read_sittings(&conn, "1").await,
            r#"["dinner"]"#,
            "an unread partial must not blank the stored reading"
        );
    }

    /// A fresh reading still replaces a stored one, so a re-read that narrows a dish
    /// takes effect rather than being frozen out by the merge.
    #[tokio::test]
    async fn a_fresh_reading_replaces_the_stored_sittings() {
        let conn = conn().await;
        let mut r = sample();
        r.sittings = vec![
            recipe_core::Sitting::Breakfast,
            recipe_core::Sitting::Lunch,
            recipe_core::Sitting::Dinner,
        ];
        upsert(&conn, &r, 1).await.unwrap();

        r.sittings = vec![recipe_core::Sitting::Dinner];
        upsert(&conn, &r, 2).await.unwrap();
        assert_eq!(read_sittings(&conn, "1").await, r#"["dinner"]"#);
    }


    // --- Every column the schema declares, the sole writer fills (#161, #176/3) ---
    //
    // The two tests below are the generic form of a bug this repo shipped, and the
    // reason they read the column list out of the live schema rather than from a
    // hand-written list: the hand-written version is the version that failed.
    // Migration 0014 added `recipes.equipment`, `derive` computed the reading, and
    // `upsert` — the only writer of this table — never named the column, so the
    // reading was recomputed and thrown away on every run for months and all 790 rows
    // read `'[]'`. `fully_timed` (#171) came one review away from the same fate. A
    // per-migration round-trip test catches this only when whoever adds the column
    // remembers to write one; `PRAGMA table_info` puts the next column in scope the
    // moment its migration lands, whether or not anybody thought about it.

    /// Columns whose default is an *expression*, so "is it still at its default" is
    /// not a meaningful question about them — the default computes a real value
    /// rather than standing in for one the writer forgot.
    ///
    /// Pinned rather than merely detected, so a new expression-defaulted column has
    /// to be exempted in a diff a human reads instead of quietly exempting itself.
    const EXPRESSION_DEFAULTS: &[&str] = &[
        // `(unixepoch())` — write time, correctly left to the schema on insert and set
        // explicitly on update. Bookkeeping, not a derived value that could be
        // computed and dropped.
        "fetched_at",
    ];

    /// Whether a `dflt_value` is a constant the column *sits at* rather than an
    /// expression that computes one.
    ///
    /// SQLite hands `DEFAULT (unixepoch())` back from `PRAGMA table_info` as
    /// `unixepoch()` — the parentheses it was written with are gone — so "is it an
    /// expression" has to be decided by what a literal looks like, not by punctuation.
    fn is_literal(default: &str) -> bool {
        let d = default.trim();
        d.eq_ignore_ascii_case("null")
            || d.parse::<f64>().is_ok()
            || (d.starts_with('\'') && d.ends_with('\'') && d.len() >= 2)
            || d.to_ascii_lowercase().starts_with("x'")
    }

    /// `(column, default)` for every column of `recipes`, straight from the live
    /// schema. `None` means no `DEFAULT` clause, so the column's default is NULL.
    async fn schema_columns(conn: &Connection) -> Vec<(String, Option<String>)> {
        let mut rows = conn.query("PRAGMA table_info(recipes)", ()).await.unwrap();
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            out.push((
                row.get::<String>(1).unwrap(),
                row.get::<Option<String>>(4).unwrap(),
            ));
        }
        assert!(
            !out.is_empty(),
            "PRAGMA table_info(recipes) returned nothing"
        );
        out
    }

    /// The whole stored row as `(column, value)`, each value rendered as the SQL
    /// literal the schema would have written — so it compares directly against
    /// `PRAGMA table_info`'s `dflt_value`, which is also SQL literal text.
    async fn whole_row(conn: &Connection, id: &str) -> Vec<(String, String)> {
        let mut rows = conn
            .query(
                "SELECT * FROM recipes WHERE source = 'themealdb' AND id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let names: Vec<String> = (0..rows.column_count())
            .map(|i| rows.column_name(i).unwrap().to_owned())
            .collect();
        let row = rows.next().await.unwrap().expect("the row must exist");
        names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let value = match row.get_value(i as i32).unwrap() {
                    libsql::Value::Null => "NULL".to_owned(),
                    libsql::Value::Integer(n) => n.to_string(),
                    libsql::Value::Real(f) => f.to_string(),
                    libsql::Value::Text(t) => format!("'{}'", t.replace('\'', "''")),
                    libsql::Value::Blob(b) => format!("x'{}'", hex::encode(b)),
                };
                (name, value)
            })
            .collect()
    }

    /// A recipe with something to say about **every** column of `recipes`: both
    /// readings present and aligned, every step timed, every stated line weighable,
    /// every nullable field supplied.
    ///
    /// When a new column makes this fixture insufficient the test below fails, and the
    /// fix is to populate the column here. That is the forcing function, not an
    /// obstacle: a column nothing can populate is a column nothing writes.
    fn fully_populated() -> Recipe {
        let mut r = weighed(500.0, 209.0, 4);
        r.title = "Full".into();
        r.image = Some("https://img.test/full.jpg".into());
        r.category = Some("Starter".into());
        r.area = Some("Thai".into());
        r.tags = vec!["easy".into()];
        r.instructions = "Boil it.".into();
        r.steps = vec![cook_step(1, Some(60), &[]), cook_step(2, Some(120), &[1])];
        r.equipment = vec![recipe_core::equipment::RequiredEquipment {
            item: "saucepan".into(),
        }];
        r.source_url = Some("https://example.test/full".into());
        r.video_url = Some("https://example.test/full.mp4".into());
        r
    }

    /// The same, populated **differently** in every column — so a column an update
    /// fails to carry shows up as a value that did not move.
    ///
    /// The two flags go the other way here (one untimed step, one stated-but-unweighable
    /// line), because they are the only columns whose populated value is a single bit:
    /// "differently populated" for them means the opposite bit, and it covers the
    /// direction [`fully_populated`] cannot.
    fn differently_populated() -> Recipe {
        let mut r = weighed(300.0, 90.0, 2);
        r.title = "Other".into();
        r.image = Some("https://img.test/other.jpg".into());
        r.category = Some("Dessert".into());
        r.area = Some("British".into());
        r.tags = vec!["slow".into()];
        r.instructions = "Bake it.".into();
        // One step with no duration: a real total, but only a lower bound.
        r.steps = vec![cook_step(1, Some(900), &[]), cook_step(2, None, &[1])];
        r.equipment = vec![recipe_core::equipment::RequiredEquipment {
            item: "oven".into(),
        }];
        // A second line stating a number nothing can weigh: the counted line still
        // gives a total, but the total is only a floor.
        r.ingredients.push(Ingredient {
            name: "cinnamon".into(),
            measure: Some("2 sticks".into()),
            structured: Some(recipe_core::StructuredMeasure {
                item: "cinnamon".into(),
                amount: Some(recipe_core::Amount::Quantified {
                    quantity: recipe_core::Quantity::Exact { value: 2.0 },
                    unit: Some("stick".into()),
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        });
        r.nutrition.push(recipe_core::FoodEnergy {
            kcal_per_100g: 247.0,
            grams_per_unit: None,
        });
        r.source_url = Some("https://example.test/other".into());
        r.video_url = Some("https://example.test/other.mp4".into());
        r
    }

    /// **The insert path names every column.** Write a recipe that has a value for
    /// every column, then assert none came out still holding the default its migration
    /// gave it. A column the sole writer never names is exactly the #161 shape: `'[]'`
    /// (or NULL, or `0`) forever, on every row, invisibly.
    #[tokio::test]
    async fn upsert_fills_every_column_the_schema_declares() {
        let conn = conn().await;
        upsert(&conn, &fully_populated(), 7).await.unwrap();

        let stored: std::collections::HashMap<String, String> =
            whole_row(&conn, "1").await.into_iter().collect();

        let mut exempt = Vec::new();
        for (name, default) in schema_columns(&conn).await {
            if default.as_deref().is_some_and(|d| !is_literal(d)) {
                exempt.push(name);
                continue;
            }
            let value = stored.get(&name).expect("every column is in the row");
            // No DEFAULT clause means the column defaults to NULL.
            let default = default.unwrap_or_else(|| "NULL".to_owned());
            assert_ne!(
                value, &default,
                "recipes.{name} is still at its schema default ({default}) after the \
                 sole writer wrote a recipe that has a value for it. `upsert` does not \
                 name the column, so it is derived and thrown away on every run (#161) \
                 — add it to the INSERT column list *and* to the ON CONFLICT SET."
            );
        }

        assert_eq!(
            exempt, EXPRESSION_DEFAULTS,
            "a column with an expression default has appeared or gone. That exempts it \
             from the check above, so the set is pinned: add it to EXPRESSION_DEFAULTS \
             along with the reason its default computes a value rather than standing in \
             for one the writer forgot."
        );
    }

    /// **And so does the update path.** A column named in the INSERT but missing from
    /// `ON CONFLICT DO UPDATE SET` is right exactly once — on the row's first write —
    /// and stale for every derive after it. That is harder to notice than #161 was,
    /// because the column is not empty, only old.
    ///
    /// Both writes are fully populated, so every merge-non-empty guard in the SET
    /// passes and every column is *supposed* to move. Whatever did not move was not
    /// carried.
    #[tokio::test]
    async fn upsert_carries_every_column_on_update_too() {
        let conn = conn().await;
        upsert(&conn, &fully_populated(), 7).await.unwrap();
        let before: std::collections::HashMap<String, String> =
            whole_row(&conn, "1").await.into_iter().collect();

        upsert(&conn, &differently_populated(), 8).await.unwrap();

        for (name, value) in whole_row(&conn, "1").await {
            // The key is the key, and `fetched_at` is a clock both writes can read
            // inside the same second.
            if ["source", "id", "fetched_at"].contains(&name.as_str()) {
                continue;
            }
            assert_ne!(
                &value,
                before.get(&name).expect("every column is in both rows"),
                "recipes.{name} did not change when a wholly different recipe was \
                 written over it — the column is missing from the ON CONFLICT SET, so \
                 it keeps whatever the row's first write put there and no later derive \
                 can correct it."
            );
        }
    }
}
