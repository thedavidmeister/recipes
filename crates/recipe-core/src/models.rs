//! Normalized recipe types shared across all sources.
//!
//! Every source (TheMealDB, arbitrary schema.org pages, future APIs) maps its
//! own payload onto these types so the frontend sees one consistent shape.

use serde::{Deserialize, Serialize};

/// A single ingredient line. `measure` is the quantity/unit when the source
/// provides it separately (TheMealDB does); free-text sources fold it into
/// `name`.
///
/// These types derive both `Serialize` and `Deserialize` so they round-trip
/// across the wire (WASM ⇄ JS ⇄ backend) and in and out of Turso.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ingredient {
    pub name: String,
    pub measure: Option<String>,
}

/// A lightweight recipe listing, as returned by search/browse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeSummary {
    /// Source-specific identifier (opaque to the frontend).
    pub id: String,
    /// Which source this came from, e.g. `"themealdb"`.
    pub source: String,
    pub title: String,
    pub image: Option<String>,
    pub category: Option<String>,
    /// Cuisine / region of origin.
    pub area: Option<String>,
}

/// A fully-resolved recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub source: String,
    pub title: String,
    pub image: Option<String>,
    pub category: Option<String>,
    pub area: Option<String>,
    pub tags: Vec<String>,
    pub ingredients: Vec<Ingredient>,
    pub instructions: String,
    /// Canonical URL of the recipe on its origin site, when known.
    pub source_url: Option<String>,
    pub video_url: Option<String>,
}
