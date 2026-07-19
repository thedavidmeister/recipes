//! Normalized recipe types shared across all sources.
//!
//! Every source (TheMealDB today, future APIs) maps its own payload onto these
//! types so the frontend sees one consistent shape.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{Amount, Quantity, StructuredMeasure};

    /// An un-enriched line must serialize exactly as it did before the field
    /// existed — no `"structured"` key — so the 700-odd rows already in Turso
    /// don't churn and old rows still read back.
    #[test]
    fn structured_none_is_omitted_and_absent_deserializes_to_none() {
        let raw = Ingredient {
            name: "flour".into(),
            measure: Some("1 cup".into()),
            structured: None,
        };
        let json = serde_json::to_string(&raw).unwrap();
        assert!(
            !json.contains("structured"),
            "an un-enriched line must not write a structured key: {json}"
        );

        // A row stored before this field existed (no key at all) reads back None.
        let old: Ingredient =
            serde_json::from_str(r#"{"name":"flour","measure":"1 cup"}"#).unwrap();
        assert_eq!(old.structured, None);
    }

    /// A populated reading round-trips intact, with the raw text alongside it —
    /// parse-but-preserve, and the storage shape acceptance #1 asks for.
    #[test]
    fn structured_some_round_trips_beside_the_raw_text() {
        let ing = Ingredient {
            name: "chicken thighs".into(),
            measure: Some("500 g".into()),
            structured: Some(StructuredMeasure {
                item: "chicken thighs".into(),
                amount: Some(Amount::Quantified {
                    quantity: Quantity::Exact { value: 500.0 },
                    unit: Some("g".into()),
                    size: None,
                }),
                preparation: Some("diced".into()),
                note: None,
            }),
        };
        let back: Ingredient = serde_json::from_str(&serde_json::to_string(&ing).unwrap()).unwrap();
        assert_eq!(ing, back);
        // Raw stays the source of truth next to the enrichment.
        assert_eq!(back.measure.as_deref(), Some("500 g"));
    }
}
