//! Server-driven ingest: pull every source's catalog into the corpus.
//!
//! The client no longer decides *what* to ingest (a search). It hits one
//! trigger; the server dispatches to each adapter's [`catalog`], fetches every
//! URL, normalizes it, and stores what comes back. There is no query — the corpus
//! becomes the union of every source's whole catalog, which is what `pick` (the
//! walk) wants to wander. See #49.
//!
//! The two I/O boundaries are traits so the engine can be exercised against a
//! fixture adapter with no network and no database — the same shape `recipe-walk`
//! uses. [`Fetcher`] is HTTP in production (SSRF-guarded via [`crate::proxy`]) and
//! a canned map in tests; [`Sink`] is Turso in production and an in-memory
//! collector in tests. The [`sync`] engine is generic over both, so it is pure
//! control flow with the effects injected.
//!
//! [`catalog`]: recipe_core::adapters::Adapter::catalog

use std::future::Future;

use recipe_core::adapters::{Adapter, Ingested};
use recipe_core::Recipe;
use serde::Serialize;
use url::Url;

/// A fetched document — what a [`Fetcher`] hands back.
pub struct Fetched {
    /// The URL after redirects — normalization reads ids/source off it.
    pub final_url: String,
    pub content_type: Option<String>,
    pub body: String,
}

/// Fetches one catalog URL. Production is SSRF-guarded HTTP; a fixture returns
/// canned bodies so the sync can be tested offline.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> impl Future<Output = anyhow::Result<Fetched>>;
}

/// Stores one complete recipe (both halves). Production writes Turso; a fixture
/// collects in memory. Idempotency is the store's job — the real one upserts on
/// `(source, id)`, so re-syncing overwrites rather than duplicating.
pub trait Sink {
    fn store(
        &self,
        item: &Ingested,
        content_type: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>>;
}

/// What a sync did — returned to the client and worth logging.
#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    /// Catalog URLs fetched successfully.
    pub fetched: usize,
    /// Complete recipes stored (upserts — re-syncing is idempotent).
    pub stored: usize,
    /// Per-URL problems, so a partial sync reports what it could not do rather
    /// than failing the whole run for one bad fetch.
    pub failures: Vec<Failure>,
}

/// One thing a sync could not do, kept with the URL it happened on.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub url: String,
    pub error: String,
}

/// Pull every adapter's catalog through `fetcher` into `sink`.
///
/// The adapter that *listed* a URL is the one that normalizes its response — no
/// host lookup, because the sync already knows the source. One bad fetch or store
/// is recorded and the run continues; the corpus is best-effort-complete, not
/// all-or-nothing.
pub async fn sync<F: Fetcher, S: Sink>(adapters: &[Adapter], fetcher: &F, sink: &S) -> SyncReport {
    let mut report = SyncReport::default();
    for adapter in adapters {
        for url in (adapter.catalog)() {
            let doc = match fetcher.fetch(&url).await {
                Ok(doc) => doc,
                Err(e) => {
                    report.failures.push(Failure {
                        url,
                        error: format!("fetch: {e}"),
                    });
                    continue;
                }
            };
            let Ok(parsed) = Url::parse(&doc.final_url) else {
                report.failures.push(Failure {
                    url,
                    error: format!("unparseable final url: {}", doc.final_url),
                });
                continue;
            };
            report.fetched += 1;
            for item in (adapter.normalize)(&parsed, &doc.body) {
                // Only complete recipes are worth storing: a header-only listing
                // (TheMealDB's `filter.php`) has nothing to contribute to the
                // corpus, even though it is fine to render elsewhere.
                if !is_complete(&item.recipe) {
                    continue;
                }
                match sink.store(&item, doc.content_type.as_deref()).await {
                    Ok(()) => report.stored += 1,
                    Err(e) => report.failures.push(Failure {
                        url: url.clone(),
                        error: format!("store {}/{}: {e}", item.recipe.source, item.recipe.id),
                    }),
                }
            }
        }
    }
    report
}

/// A recipe carries what a corpus is for. TheMealDB's `filter.php` returns header
/// fields only — browsing Seafood yields 82 recipes with no ingredients or
/// instructions, fine to show and pointless to store.
fn is_complete(recipe: &Recipe) -> bool {
    !recipe.instructions.trim().is_empty() && !recipe.ingredients.is_empty()
}

/// The production [`Fetcher`]: SSRF-guarded HTTP through [`crate::proxy`].
pub struct ProxyFetcher<'a> {
    pub http: &'a reqwest::Client,
}

impl Fetcher for ProxyFetcher<'_> {
    async fn fetch(&self, url: &str) -> anyhow::Result<Fetched> {
        let resp = crate::proxy::fetch_url(self.http, url)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Fetched {
            final_url: resp.final_url,
            content_type: resp.content_type,
            body: resp.body,
        })
    }
}

/// The production [`Sink`]: both halves into Turso via [`crate::recipes`].
pub struct TursoSink<'a> {
    pub conn: &'a libsql::Connection,
}

impl Sink for TursoSink<'_> {
    async fn store(&self, item: &Ingested, content_type: Option<&str>) -> anyhow::Result<()> {
        crate::recipes::store(self.conn, item, content_type).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::adapters::Adapter;
    use recipe_core::{Ingredient, Recipe};
    use std::collections::HashMap;
    use std::sync::Mutex;

    // --- A fixture adapter: its catalog is four URLs, its normalize turns a
    // one-line body into one complete recipe (blank body → nothing). ------------

    fn fixture_catalog() -> Vec<String> {
        ["fix://soup", "fix://stew", "fix://blank", "fix://gone"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn fixture_normalize(url: &Url, body: &str) -> Vec<Ingested> {
        if body.trim().is_empty() {
            return vec![]; // a listing with no recipes — a normal, empty outcome
        }
        let recipe = Recipe {
            id: body.to_string(),
            source: "fixture".to_string(),
            title: body.to_string(),
            image: None,
            category: None,
            area: None,
            tags: vec![],
            ingredients: vec![Ingredient {
                name: "water".to_string(),
                measure: None,
            }],
            instructions: "Cook it.".to_string(),
            source_url: None,
            video_url: None,
        };
        vec![Ingested {
            recipe,
            raw: body.to_string(),
            fetched_from: url.to_string(),
        }]
    }

    fn fixture_handles(_: &str) -> bool {
        false
    }

    const FIXTURE: Adapter = Adapter {
        id: "fixture",
        handles: fixture_handles,
        normalize: fixture_normalize,
        catalog: fixture_catalog,
    };

    struct FixtureFetcher {
        docs: HashMap<String, String>,
    }

    impl Fetcher for FixtureFetcher {
        async fn fetch(&self, url: &str) -> anyhow::Result<Fetched> {
            match self.docs.get(url) {
                Some(body) => Ok(Fetched {
                    final_url: url.to_string(),
                    content_type: Some("application/json".to_string()),
                    body: body.clone(),
                }),
                None => anyhow::bail!("no fixture for {url}"),
            }
        }
    }

    #[derive(Default)]
    struct MemorySink {
        stored: Mutex<Vec<Recipe>>,
    }

    impl Sink for MemorySink {
        async fn store(&self, item: &Ingested, _content_type: Option<&str>) -> anyhow::Result<()> {
            self.stored.lock().unwrap().push(item.recipe.clone());
            Ok(())
        }
    }

    fn fetcher_with(pairs: &[(&str, &str)]) -> FixtureFetcher {
        FixtureFetcher {
            docs: pairs
                .iter()
                .map(|(u, b)| (u.to_string(), b.to_string()))
                .collect(),
        }
    }

    #[tokio::test]
    async fn pulls_the_catalog_stores_complete_and_reports_the_rest() {
        // soup/stew have bodies (→ complete), blank is an empty listing, gone is
        // missing from the fetcher (→ a fetch failure).
        let fetcher = fetcher_with(&[
            ("fix://soup", "Soup"),
            ("fix://stew", "Stew"),
            ("fix://blank", ""),
        ]);
        let sink = MemorySink::default();

        let report = sync(&[FIXTURE], &fetcher, &sink).await;

        assert_eq!(report.stored, 2, "soup and stew are complete");
        assert_eq!(report.fetched, 3, "soup, stew, blank fetched; gone failed");
        assert_eq!(report.failures.len(), 1, "only the missing url failed");
        assert_eq!(report.failures[0].url, "fix://gone");

        let titles: Vec<String> = sink
            .stored
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.title.clone())
            .collect();
        assert_eq!(titles, ["Soup", "Stew"]);
    }

    #[tokio::test]
    async fn an_empty_catalog_stores_nothing() {
        fn empty_catalog() -> Vec<String> {
            vec![]
        }
        const CLAIMLESS: Adapter = Adapter {
            id: "claimless",
            handles: fixture_handles,
            normalize: fixture_normalize,
            catalog: empty_catalog,
        };
        let report = sync(&[CLAIMLESS], &fetcher_with(&[]), &MemorySink::default()).await;
        assert_eq!(report.stored, 0);
        assert_eq!(report.fetched, 0);
        assert!(report.failures.is_empty());
    }

    #[tokio::test]
    async fn a_store_error_is_recorded_not_fatal() {
        struct FailingSink;
        impl Sink for FailingSink {
            async fn store(&self, _: &Ingested, _: Option<&str>) -> anyhow::Result<()> {
                anyhow::bail!("disk full")
            }
        }
        let fetcher = fetcher_with(&[
            ("fix://soup", "Soup"),
            ("fix://stew", "Stew"),
            ("fix://blank", ""),
            ("fix://gone", ""),
        ]);
        let report = sync(&[FIXTURE], &fetcher, &FailingSink).await;
        assert_eq!(report.stored, 0, "every store failed");
        assert_eq!(report.fetched, 4, "but every fetch succeeded");
        assert_eq!(report.failures.len(), 2, "soup and stew failed to store");
        assert!(report
            .failures
            .iter()
            .all(|f| f.error.contains("disk full")));
    }
}
