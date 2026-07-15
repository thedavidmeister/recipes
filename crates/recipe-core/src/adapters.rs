//! Adapters: the only way into the corpus.
//!
//! The corpus is a **cache of normalized recipes from sources we support** — not
//! user input. So ingestion starts from an adapter for a *known* source, and a
//! document from an unknown host **fails closed** rather than being parsed
//! best-effort. Supporting arbitrary domains buys nothing when adapters are
//! needed to normalize well anyway: it only yields mediocre data for sites
//! nobody has looked at, and it means normalizing pages an attacker authored.
//!
//! The host is **derived from the URL here**, never supplied by the caller: if a
//! caller passed the host alongside the URL the two could disagree, and a claim
//! of `www.themealdb.com` for `https://evil.example/x` would pass the gate while
//! the adapter normalized something else entirely. The gate must read the host
//! off the same URL the adapter is handed.

use url::Url;

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

/// Why a document was not ingested. Distinct kinds, so callers branch on the
/// variant rather than matching on a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The URL could not be parsed, or carries no host.
    InvalidUrl(String),
    /// No adapter claims that host, so it is not a source we ingest.
    UnsupportedSource { host: String },
}

impl core::fmt::Display for IngestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IngestError::InvalidUrl(url) => write!(f, "invalid url: {url}"),
            IngestError::UnsupportedSource { host } => write!(f, "unsupported source: {host}"),
        }
    }
}

/// Normalize a fetched document, failing closed unless an adapter claims the
/// URL's host.
///
/// This is the single entry point for ingestion — routing through it is what
/// keeps unknown sources out of the corpus.
pub fn normalize(url: &str, body: &str) -> Result<Vec<Recipe>, IngestError> {
    let parsed = Url::parse(url).map_err(|_| IngestError::InvalidUrl(url.to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| IngestError::InvalidUrl(url.to_string()))?;

    match adapter_for(host) {
        Some(adapter) => Ok((adapter.normalize)(url, body)),
        None => Err(IngestError::UnsupportedSource {
            host: host.to_string(),
        }),
    }
}

/// Whether the URL's host is a source we ingest.
pub fn is_supported(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| adapter_for(h).is_some()))
        .unwrap_or(false)
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
        assert_eq!(
            normalize("https://example.com/x", "<html></html>"),
            Err(IngestError::UnsupportedSource {
                host: "example.com".into()
            })
        );
    }

    /// The host is read off the URL, so a caller cannot claim a supported source
    /// for someone else's document — the reason this crate parses the URL rather
    /// than accepting a `host` argument.
    #[test]
    fn host_comes_from_the_url_not_the_caller() {
        let meals = r#"{"meals":[{"idMeal":"1","strMeal":"Evil","strInstructions":"x"}]}"#;
        assert_eq!(
            normalize("https://evil.example/api/themealdb.com/search.php", meals),
            Err(IngestError::UnsupportedSource {
                host: "evil.example".into()
            }),
            "a themealdb-looking path on another host must not be ingested"
        );
        // Nor via userinfo/subdomain lookalikes.
        for url in [
            "https://www.themealdb.com.evil.example/x",
            "https://evil.example/?q=www.themealdb.com",
            "https://user@evil.example/x",
        ] {
            assert!(
                matches!(
                    normalize(url, meals),
                    Err(IngestError::UnsupportedSource { .. })
                ),
                "{url} must not be ingested"
            );
        }
    }

    #[test]
    fn unparseable_urls_are_rejected_distinctly() {
        assert_eq!(
            normalize("not-a-url", "{}"),
            Err(IngestError::InvalidUrl("not-a-url".into()))
        );
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
        let recipes = normalize(
            "https://www.themealdb.com/api/json/v1/1/lookup.php?i=1",
            json,
        )
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
            "https://www.themealdb.com/api/json/v1/1/categories.php",
            categories,
        )
        .expect("supported");
        assert!(recipes.is_empty());
    }
}
