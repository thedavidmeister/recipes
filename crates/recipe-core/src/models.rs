//! Normalized recipe types shared across all sources.
//!
//! Every source (TheMealDB, arbitrary schema.org pages, future APIs) maps its
//! own payload onto these types so the frontend sees one consistent shape.

use serde::{Deserialize, Serialize};

use crate::measure::StructuredMeasure;

/// A single ingredient line. `measure` is the quantity/unit when the source
/// provides it separately (TheMealDB does); free-text sources fold it into
/// `name`.
///
/// These types derive both `Serialize` and `Deserialize` so they round-trip
/// across the wire (WASM ⇄ JS ⇄ backend) and in and out of Turso.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ingredient {
    pub name: String,
    pub measure: Option<String>,
    /// The LLM's structured reading of this line (#11), populated at ingestion by
    /// the backend's `enrich` step. The raw `name`/`measure` above stay the source
    /// of truth — this is an enrichment the UI can use for scaling and unit
    /// conversion, and falls back from if the model was wrong (parse-but-preserve).
    ///
    /// `None` until enrichment runs (or when no API key is configured — it degrades
    /// rather than blocking ingestion). `#[serde(default)]` so rows stored before
    /// this field existed still deserialize; `skip_serializing_if` so an un-enriched
    /// line stores exactly as it did before, no churn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<StructuredMeasure>,
}

/// A normalized recipe from any source.
///
/// Some arrive only partially populated — a browse/listing result (e.g.
/// TheMealDB `filter.php`) may carry just `id`, `source`, `title`, and `image`,
/// with the rest empty. Absent detail is represented as empty (`""` / `[]` /
/// `None`), not as a distinct "unknown".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// Source-specific identifier (opaque to the frontend).
    pub id: String,
    /// Which source this came from, e.g. `"themealdb"`.
    pub source: String,
    pub title: String,
    pub image: Option<String>,
    pub category: Option<String>,
    /// Cuisine / region of origin.
    pub area: Option<String>,
    pub tags: Vec<String>,
    pub ingredients: Vec<Ingredient>,
    pub instructions: String,
    /// Canonical URL of the recipe on its origin site, when known.
    pub source_url: Option<String>,
    pub video_url: Option<String>,
}
