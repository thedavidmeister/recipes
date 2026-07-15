//! Adapters: the only way into the corpus.
//!
//! The corpus is a **cache of normalized recipes from sources we support** — not
//! user input. So ingestion starts from an adapter for a *known* source, and a
//! document from an unknown host **fails closed** rather than being parsed
//! best-effort. Supporting arbitrary domains buys nothing when adapters are
//! needed to normalize well anyway: it only yields mediocre data for sites
//! nobody has looked at, and it means normalizing pages an attacker authored.
//!
//! Callers pass an already-parsed `host`. This crate deliberately does not parse
//! URLs: the browser (`new URL()`) and the backend (`url`) both already have a
//! real parser, and pulling `url`/`idna` in for host matching would bloat the
//! wasm bundle for nothing.

use crate::models::Recipe;
use crate::{schema_org, themealdb};

/// A source we support.
pub struct Adapter {
    /// Recorded as [`Recipe::source`], and how a stored row says where it came
    /// from.
    pub id: &'static str,
    /// Whether this adapter claims `host`.
    pub handles: fn(host: &str) -> bool,
    /// Normalize a document fetched from this source. Empty when the document
    /// carries no recipes (a category listing, say).
    pub normalize: fn(url: &str, body: &str) -> Vec<Recipe>,
}

/// Every supported source, in match order.
pub const ADAPTERS: &[Adapter] = &[
    Adapter {
        id: themealdb::SOURCE,
        handles: themealdb::handles,
        normalize: themealdb::normalize_document,
    },
    Adapter {
        id: schema_org::SOURCE,
        handles: schema_org::handles,
        normalize: schema_org::normalize_document,
    },
];

/// The adapter claiming `host`, if any.
pub fn adapter_for(host: &str) -> Option<&'static Adapter> {
    ADAPTERS.iter().find(|a| (a.handles)(host))
}

/// No adapter claims that host, so it is not a source we ingest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedSource {
    pub host: String,
}

/// Normalize a fetched document, failing closed when no adapter claims `host`.
///
/// This is the single entry point for ingestion — routing through it is what
/// keeps unknown sources out of the corpus.
pub fn normalize(host: &str, url: &str, body: &str) -> Result<Vec<Recipe>, UnsupportedSource> {
    match adapter_for(host) {
        Some(adapter) => Ok((adapter.normalize)(url, body)),
        None => Err(UnsupportedSource {
            host: host.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themealdb_is_supported() {
        let a = adapter_for("www.themealdb.com").expect("themealdb adapter");
        assert_eq!(a.id, "themealdb");
    }

    #[test]
    fn unknown_hosts_fail_closed() {
        // The point of the design: an arbitrary site is not a source.
        for host in ["example.com", "evil.test", "bbcgoodfood.com", ""] {
            assert!(adapter_for(host).is_none(), "{host} must not be supported");
        }
        let err = normalize("example.com", "https://example.com/x", "<html></html>")
            .expect_err("an unknown host must not be ingested");
        assert_eq!(err.host, "example.com");
    }

    /// schema.org is kept but demoted — it is no longer the way in. It claims no
    /// host until domains are explicitly allowlisted into it.
    #[test]
    fn schema_org_claims_nothing_by_default() {
        for host in ["example.com", "www.themealdb.com", "recipes.test"] {
            assert!(!(schema_org::handles)(host));
        }
    }

    #[test]
    fn themealdb_document_normalizes_through_the_registry() {
        let json = r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1 slice"}]}"#;
        let recipes = normalize("www.themealdb.com", "https://www.themealdb.com/api/json/v1/1/lookup.php?i=1", json)
            .expect("supported");
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].title, "Toast");
        assert_eq!(recipes[0].source, "themealdb");
    }

    /// A document with no recipes is a normal outcome, not an error — only an
    /// unknown *source* is an error.
    #[test]
    fn supported_source_with_no_recipes_is_ok_and_empty() {
        let categories = r#"{"categories":[{"strCategory":"Beef"}]}"#;
        let recipes = normalize(
            "www.themealdb.com",
            "https://www.themealdb.com/api/json/v1/1/categories.php",
            categories,
        )
        .expect("supported");
        assert!(recipes.is_empty());
    }
}
