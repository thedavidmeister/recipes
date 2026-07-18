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

/// Persists one fetched payload into `raw_imports` — raw only. `recipes` is
/// derived and written solely by [`crate::derive`], not here (the write path is
/// decoupled by table). Production writes Turso; a fixture collects in memory.
/// Idempotency is the store's job — the real one upserts on `(source, id)`,
/// guarded by `run_id`, so re-syncing overwrites rather than duplicating and a
/// stale run cannot clobber a newer one.
pub trait Sink {
    fn store_raw(
        &self,
        item: &Ingested,
        content_type: Option<&str>,
        run_id: i64,
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
/// The adapter that *listed* a URL is the one that normalizes its response. That
/// does not make the host trustworthy, so it is checked twice against
/// [`Adapter::handles`], failing closed both times:
///
/// - **before fetching**, because an adapter's catalog must only name hosts it
///   claims — a catalog that wandered off-source should not spend a request;
/// - **after fetching**, because the fetch follows redirects: the body may have
///   come from somewhere else entirely, and normalizing it as this adapter would
///   attribute a stranger's data to this source. The gate has to read the host off
///   the same URL the adapter is handed.
///
/// One bad fetch or store is recorded and the run continues; the corpus is
/// best-effort-complete, not all-or-nothing.
///
/// [`Adapter::handles`]: recipe_core::adapters::Adapter::handles
pub async fn sync<F: Fetcher, S: Sink>(
    adapters: &[Adapter],
    fetcher: &F,
    sink: &S,
    run_id: i64,
) -> SyncReport {
    let mut report = SyncReport::default();
    for adapter in adapters {
        for url in (adapter.catalog)() {
            if !claims(adapter, &url) {
                report.failures.push(Failure {
                    error: format!("catalog url is not a host {} claims", adapter.id),
                    url,
                });
                continue;
            }
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
            if !claims(adapter, &doc.final_url) {
                report.failures.push(Failure {
                    error: format!(
                        "redirected off-source to {} — not a host {} claims",
                        doc.final_url, adapter.id
                    ),
                    url,
                });
                continue;
            }
            report.fetched += 1;
            for item in (adapter.normalize)(&parsed, &doc.body) {
                // Only complete recipes are worth storing: a header-only listing
                // (TheMealDB's `filter.php`) has nothing to contribute to the
                // corpus, even though it is fine to render elsewhere.
                if !is_complete(&item.recipe) {
                    continue;
                }
                match sink
                    .store_raw(&item, doc.content_type.as_deref(), run_id)
                    .await
                {
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

/// Does `adapter` claim the host of `url`?
///
/// Fails closed on anything it cannot read as a claimed host — an unparseable
/// URL, or one with no host at all.
fn claims(adapter: &Adapter, url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|host| (adapter.handles)(host)))
        .unwrap_or(false)
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

/// The production [`Sink`]: the fetched payload into `raw_imports` via
/// [`crate::recipes::store_raw`]. `recipes` is left to [`crate::derive`].
pub struct TursoSink<'a> {
    pub conn: &'a libsql::Connection,
}

impl Sink for TursoSink<'_> {
    async fn store_raw(
        &self,
        item: &Ingested,
        content_type: Option<&str>,
        run_id: i64,
    ) -> anyhow::Result<()> {
        crate::recipes::store_raw(self.conn, item, content_type, run_id).await
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
                structured: None,
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

    /// The hosts of the fixture catalog (`fix://soup` parses to host `soup`). A
    /// real adapter claims its source's hosts; this one has to too, because the
    /// sync gates on `handles` both before fetching and after redirects.
    fn fixture_handles(host: &str) -> bool {
        matches!(host, "soup" | "stew" | "blank" | "gone")
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
        async fn store_raw(
            &self,
            item: &Ingested,
            _content_type: Option<&str>,
            _run_id: i64,
        ) -> anyhow::Result<()> {
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

        let report = sync(&[FIXTURE], &fetcher, &sink, 1).await;

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

    /// A fetch that lands somewhere the adapter does not claim must not be
    /// normalized as that adapter — the body here would parse perfectly well, and
    /// that is precisely the danger: it would enter the corpus attributed to a
    /// source it never came from.
    #[tokio::test]
    async fn a_redirect_off_source_is_refused() {
        struct RedirectingFetcher;
        impl Fetcher for RedirectingFetcher {
            async fn fetch(&self, _url: &str) -> anyhow::Result<Fetched> {
                Ok(Fetched {
                    final_url: "https://evil.example/x".to_string(),
                    content_type: Some("application/json".to_string()),
                    body: "Soup".to_string(), // would normalize fine — that is the point
                })
            }
        }
        let sink = MemorySink::default();
        let report = sync(&[FIXTURE], &RedirectingFetcher, &sink, 1).await;

        assert_eq!(report.stored, 0, "a stranger's body must not be stored");
        assert_eq!(
            report.fetched, 0,
            "an off-source response is not a fetch we keep"
        );
        assert_eq!(
            report.failures.len(),
            4,
            "every catalog url redirected away"
        );
        assert!(report
            .failures
            .iter()
            .all(|f| f.error.contains("evil.example")));
        assert!(sink.stored.lock().unwrap().is_empty());
    }

    /// A catalog that names a host its own adapter does not claim is refused
    /// *before* a request is spent — the fetcher here panics if it is ever called.
    #[tokio::test]
    async fn an_unclaimed_catalog_url_is_refused_before_fetching() {
        fn rogue_catalog() -> Vec<String> {
            vec!["https://evil.example/x".to_string()]
        }
        const ROGUE: Adapter = Adapter {
            id: "rogue",
            handles: fixture_handles,
            normalize: fixture_normalize,
            catalog: rogue_catalog,
        };
        struct NeverFetcher;
        impl Fetcher for NeverFetcher {
            async fn fetch(&self, url: &str) -> anyhow::Result<Fetched> {
                panic!("the gate must answer before fetching, but it fetched {url}")
            }
        }
        let report = sync(&[ROGUE], &NeverFetcher, &MemorySink::default(), 1).await;
        assert_eq!(report.fetched, 0);
        assert_eq!(report.stored, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].error.contains("not a host rogue claims"));
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
        let report = sync(&[CLAIMLESS], &fetcher_with(&[]), &MemorySink::default(), 1).await;
        assert_eq!(report.stored, 0);
        assert_eq!(report.fetched, 0);
        assert!(report.failures.is_empty());
    }

    #[tokio::test]
    async fn a_store_error_is_recorded_not_fatal() {
        struct FailingSink;
        impl Sink for FailingSink {
            async fn store_raw(&self, _: &Ingested, _: Option<&str>, _: i64) -> anyhow::Result<()> {
                anyhow::bail!("disk full")
            }
        }
        let fetcher = fetcher_with(&[
            ("fix://soup", "Soup"),
            ("fix://stew", "Stew"),
            ("fix://blank", ""),
            ("fix://gone", ""),
        ]);
        let report = sync(&[FIXTURE], &fetcher, &FailingSink, 1).await;
        assert_eq!(report.stored, 0, "every store failed");
        assert_eq!(report.fetched, 4, "but every fetch succeeded");
        assert_eq!(report.failures.len(), 2, "soup and stew failed to store");
        assert!(report
            .failures
            .iter()
            .all(|f| f.error.contains("disk full")));
    }
}
