//! Turso write-gateway: persist a normalized recipe into the corpus.
//!
//! The browser produces a `Recipe` (via recipe-core WASM) and POSTs it here; the
//! backend holds the Turso *write* token (the browser never does) and upserts on
//! `(source, id)`. Reads do not come through here — the frontend reads Turso
//! directly with a read-only token.

use axum::{extract::State, http::StatusCode, Json};
use libsql::Connection;
use recipe_core::Recipe;

use crate::{error::AppError, AppState};

/// `POST /api/recipes` — validate and upsert a normalized recipe.
pub async fn create_recipe(
    State(state): State<AppState>,
    Json(recipe): Json<Recipe>,
) -> Result<StatusCode, AppError> {
    if recipe.source.trim().is_empty() || recipe.id.trim().is_empty() {
        return Err(AppError::BadRequest("source and id are required".into()));
    }
    if recipe.title.trim().is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    upsert(&state.db, &recipe)
        .await
        .map_err(|e| AppError::Internal(format!("db write failed: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
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
async fn upsert(conn: &Connection, recipe: &Recipe) -> anyhow::Result<()> {
    let tags = serde_json::to_string(&recipe.tags)?;
    let ingredients = serde_json::to_string(&recipe.ingredients)?;
    conn.execute(
        "INSERT INTO recipes
            (source, id, title, image, category, area, tags, ingredients, instructions, source_url, video_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
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
            source_url   = COALESCE(NULLIF(excluded.source_url, ''), recipes.source_url),
            video_url    = COALESCE(NULLIF(excluded.video_url, ''), recipes.video_url),
            fetched_at   = unixepoch()",
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
            }],
            instructions: "Boil.".into(),
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
        upsert(&conn, &full).await.unwrap();

        upsert(&conn, &partial()).await.unwrap();

        let (instructions, ingredients, tags, area) = read(&conn).await;
        assert_eq!(instructions, "Boil.", "instructions must survive a partial");
        assert!(
            ingredients.contains("water"),
            "ingredients must survive a partial, got {ingredients}"
        );
        assert_eq!(tags, r#"["easy"]"#, "tags must survive a partial");
        assert_eq!(area.as_deref(), Some("Italian"), "area must survive a partial");
    }

    /// The other direction still has to work: a full record fills in a partial.
    #[tokio::test]
    async fn full_upgrades_a_partial_record() {
        let conn = conn().await;

        upsert(&conn, &partial()).await.unwrap();
        let (instructions, ingredients, ..) = read(&conn).await;
        assert_eq!(instructions, "");
        assert_eq!(ingredients, "[]");

        upsert(&conn, &sample()).await.unwrap();

        let (instructions, ingredients, tags, _) = read(&conn).await;
        assert_eq!(instructions, "Boil.");
        assert!(ingredients.contains("water"));
        assert_eq!(tags, r#"["easy"]"#);
    }

    /// Merging must not freeze a field: a non-empty value still overwrites.
    #[tokio::test]
    async fn non_empty_still_overwrites() {
        let conn = conn().await;
        upsert(&conn, &sample()).await.unwrap();

        let mut revised = sample();
        revised.instructions = "Simmer gently.".into();
        revised.area = Some("French".into());
        upsert(&conn, &revised).await.unwrap();

        let (instructions, _, _, area) = read(&conn).await;
        assert_eq!(instructions, "Simmer gently.");
        assert_eq!(area.as_deref(), Some("French"));
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
        upsert(&conn, &recipe).await.unwrap();

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
        upsert(&conn, &recipe).await.unwrap();

        let mut rows = conn
            .query("SELECT count(*), max(title) FROM recipes", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<i64>(0).unwrap(), 1);
        assert_eq!(row.get::<String>(1).unwrap(), "Better Soup");
    }
}
