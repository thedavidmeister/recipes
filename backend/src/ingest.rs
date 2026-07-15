//! Ingest: fetch a document from a supported source, derive recipes, store both.
//!
//! The client **drives** ingestion — it decides what to look for — but the
//! server **performs** it. That split is why there is no WASM: the browser's
//! copy of the normalizer only ever existed to parse arbitrary pages the browser
//! had fetched itself, and the corpus no longer ingests arbitrary pages. Once
//! the server does the fetching, it already holds the bytes, and normalizing
//! them here means one normalizer instead of two, no client trust, and nothing
//! for a visitor to download.
//!
//! It also lets a source need a credential: an API key can live in a Render env
//! var, which a public SPA could never hold.
//!
//! Both halves are stored from one place: `raw_imports` (what the source said)
//! and `recipes` (what we derived). A recipe can never arrive without its raw,
//! so the derived view is always rebuildable — see [`crate::derive`].

use axum::{extract::State, Json};
use recipe_core::{adapters, Recipe};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, proxy, recipes, AppState};

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// The document to ingest. Its host must be one an adapter claims.
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    /// What the document held, for the client to render immediately.
    pub recipes: Vec<Recipe>,
    /// How many were complete enough to store.
    pub stored: usize,
}

/// `POST /api/ingest` — fetch, derive, store, and return what was found.
pub async fn ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, AppError> {
    // Fail closed before fetching: an unsupported host is not a source we
    // ingest, so there is no reason to spend a request on it. This also stops
    // the endpoint being a general-purpose fetch relay — it can only reach
    // hosts an adapter claims.
    if !adapters::is_supported(&req.url) {
        return Err(AppError::BadRequest(format!(
            "unsupported source: {}",
            req.url
        )));
    }

    // The SSRF guard still applies: adapters name hosts, DNS resolves them.
    let page = proxy::fetch_url(&state.http, &req.url).await?;

    let ingested = adapters::normalize(&page.final_url, &page.body)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let mut stored = 0;
    let mut found = Vec::with_capacity(ingested.len());
    for item in ingested {
        // Only complete recipes are worth storing: a category listing returns
        // header fields only, and a partial has nothing to contribute. They are
        // still returned, because they are worth *rendering*.
        if is_complete(&item.recipe) {
            recipes::store(&state.db, &item, page.content_type.as_deref())
                .await
                .map_err(|e| AppError::Internal(format!("db write failed: {e}")))?;
            stored += 1;
        }
        found.push(item.recipe);
    }

    Ok(Json(IngestResponse {
        recipes: found,
        stored,
    }))
}

/// A recipe carries what a corpus is for. TheMealDB's `filter.php` returns
/// header fields only — browsing Seafood yields 82 recipes with no ingredients
/// or instructions, which are fine to show and pointless to store.
fn is_complete(recipe: &Recipe) -> bool {
    !recipe.instructions.trim().is_empty() && !recipe.ingredients.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::Ingredient;

    fn recipe(instructions: &str, ingredients: Vec<Ingredient>) -> Recipe {
        Recipe {
            id: "1".into(),
            source: "themealdb".into(),
            title: "Soup".into(),
            image: None,
            category: None,
            area: None,
            tags: vec![],
            ingredients,
            instructions: instructions.into(),
            source_url: None,
            video_url: None,
        }
    }

    #[test]
    fn completeness_gates_storage_not_display() {
        let full = recipe(
            "Boil.",
            vec![Ingredient {
                name: "water".into(),
                measure: None,
            }],
        );
        assert!(is_complete(&full));

        // A category-browse shaped record: header fields only.
        assert!(!is_complete(&recipe("", vec![])));
        assert!(!is_complete(&recipe("Boil.", vec![])));
        assert!(!is_complete(&recipe(
            "   ",
            vec![Ingredient {
                name: "water".into(),
                measure: None
            }]
        )));
    }
}
