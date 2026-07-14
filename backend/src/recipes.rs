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
async fn upsert(conn: &Connection, recipe: &Recipe) -> anyhow::Result<()> {
    let tags = serde_json::to_string(&recipe.tags)?;
    let ingredients = serde_json::to_string(&recipe.ingredients)?;
    conn.execute(
        "INSERT INTO recipes
            (source, id, title, image, category, area, tags, ingredients, instructions, source_url, video_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(source, id) DO UPDATE SET
            title        = excluded.title,
            image        = excluded.image,
            category     = excluded.category,
            area         = excluded.area,
            tags         = excluded.tags,
            ingredients  = excluded.ingredients,
            instructions = excluded.instructions,
            source_url   = excluded.source_url,
            video_url    = excluded.video_url,
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
