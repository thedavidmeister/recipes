//! `POST /api/ingest` — pull every source's catalog into the corpus and derive.
//!
//! This used to take a client-supplied URL and ingest that one document —
//! "ingest is what a search does". It no longer does: the client hits a **trigger
//! with no target**, and the server dispatches to every adapter's catalog itself,
//! fetches, normalizes, and stores. There is no query; search is gone (#49).
//!
//! One trigger runs sync then derive under one `run_id`, so its writes are ordered
//! against any concurrent CLI run: `sync` fetches and writes `raw_imports`, and
//! `derive` rebuilds `recipes` from raw (reattaching whatever readings are already
//! stored). Each stage writes one table. The sync engine lives in [`crate::sync`],
//! behind [`sync::Fetcher`]/[`sync::Sink`] so it can be tested against a fixture.
//!
//! **Enrichment is not part of this path (#59).** Reading ingredient lines into
//! structure is an LLM job that runs *off* this service — an out-of-band worker
//! (`recipe-backend enrich pull|push`, driven by the enrich skill) produces the
//! readings, and the next `derive` reattaches them. There is no model here, no
//! provider credential, and no enrich step in the request: the trigger only syncs
//! and derives.
//!
//! **Machine-gated, not session-gated**: `Authorization: Bearer <INGEST_API_KEY>`
//! (see [`crate::auth::require_api_key`]). A browser session does not authorize
//! this endpoint — the client has no access to ingestion at all, which is the
//! point of #49. A schedule holds the key; nobody presses a button.

use axum::{extract::State, Json};
use libsql::Connection;
use recipe_core::adapters::{self, Adapter};
use serde::Serialize;

use crate::{derive, runs, sync, AppState};

/// What the ingest trigger did — sync's and derive's reports, so the scheduled job
/// can log fetch/derive counts from one response.
#[derive(Serialize)]
pub struct IngestReport {
    sync: sync::SyncReport,
    derive: derive::Report,
}

/// `POST /api/ingest` — trigger a server-driven corpus sync + derive.
pub async fn ingest(State(state): State<AppState>) -> Json<IngestReport> {
    // One connection for the whole trigger: this is a long job, not a request-shaped
    // one, and the two halves must write under the same run.
    let db = match state.database.connect() {
        Ok(db) => db,
        Err(e) => {
            // A scheduled trigger has nobody to show an error to, and the report is
            // the log. An empty one says plainly that nothing was synced.
            tracing::error!("ingest could not reach the database: {e}");
            return Json(IngestReport {
                sync: sync::SyncReport::default(),
                derive: derive::Report::default(),
            });
        }
    };

    let fetcher = sync::ProxyFetcher { http: &state.http };
    let sink = sync::TursoSink { conn: &db };
    Json(pipeline(&db, adapters::ADAPTERS, &fetcher, &sink).await)
}

/// One trigger, end to end: open a run, sync, derive, and close the run with what it
/// actually did.
///
/// Split out of the handler and generic over the sync boundaries for the same reason
/// [`crate::sync`] is: the whole trigger — including the status it records — is then
/// exercisable against a fixture adapter with no network.
///
/// **Both halves still degrade rather than 500 a scheduled trigger.** A dead source
/// is recorded and the sync continues; a derive that blows up leaves the previous
/// `recipes` in place. That is #146's boot ruling applied to ingest and it does not
/// change here. What changes is the *record* (#174): a trigger that fetched nothing
/// and derived nothing no longer looks, in the `runs` table, exactly like one that
/// did the whole job.
async fn pipeline<F: sync::Fetcher, S: sync::Sink>(
    db: &Connection,
    adapters: &[Adapter],
    fetcher: &F,
    sink: &S,
) -> IngestReport {
    // One run for the whole trigger, so every write it makes is ordered against a
    // concurrent CLI `enrich`/`derive` (#11 write-path hardening). Best-effort: if
    // the run row can't be opened, stamp 0 — superseded by any real run — rather
    // than 500 a scheduled trigger.
    let run_id = runs::begin(db, "ingest").await.unwrap_or_else(|e| {
        tracing::warn!("could not open a run, stamping 0: {e}");
        0
    });

    let sync = sync::sync(adapters, fetcher, sink, run_id).await;
    // A source that 502s a scraper is ordinary weather, so it is not a failed run —
    // but it is not a whole one either, and every URL it dropped is named in
    // `failures` for whoever follows the status back.
    let sync_outcome = if sync.failures.is_empty() {
        runs::Outcome::Completed
    } else {
        runs::Outcome::Partial
    };

    // Derive so `recipes` reflects the raw just synced (and reattaches whatever
    // readings the enrich worker has already stored).
    let (derive, derive_outcome) = match derive::derive(db, None, run_id).await {
        Ok(report) => (report, runs::Outcome::Completed),
        Err(e) => {
            tracing::warn!("derive step failed, leaving recipes as-is: {e}");
            (derive::Report::default(), runs::Outcome::Failed)
        }
    };

    // Close the run with the worse of the two stages. A run left open (this failing,
    // or the process dying) is the "died mid-flight" signal the runs table exists to
    // surface, and it stays reserved for that.
    let outcome = sync_outcome.worst(derive_outcome);
    if let Err(e) = runs::finish(db, run_id, outcome).await {
        tracing::warn!("could not close run {run_id}: {e}");
    }

    IngestReport { sync, derive }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_core::adapters::Ingested;
    use recipe_core::{Ingredient, Recipe};
    use std::collections::HashMap;
    use url::Url;

    /// A two-URL catalog standing in for a source. Both hosts are claimed, so the
    /// sync's before/after gates pass and the only thing under test is the outcome.
    fn fixture_catalog() -> Vec<String> {
        vec!["fix://soup".to_string(), "fix://stew".to_string()]
    }

    fn fixture_handles(host: &str) -> bool {
        matches!(host, "soup" | "stew")
    }

    fn fixture_normalize(url: &Url, body: &str) -> Vec<Ingested> {
        if body.trim().is_empty() {
            return vec![];
        }
        vec![Ingested {
            recipe: Recipe {
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
                steps: Vec::new(),
                equipment: Vec::new(),
                // Unread, like every other reading on this fixture: the outcome under
                // test is the run's, not the corpus's, and an invented figure here
                // would be a fixture asserting a calorie count no source ever gave.
                nutrition: Vec::new(),
                servings: None,
                source_url: None,
                video_url: None,
            },
            raw: body.to_string(),
            fetched_from: url.to_string(),
        }]
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

    impl sync::Fetcher for FixtureFetcher {
        async fn fetch(&self, url: &str) -> anyhow::Result<sync::Fetched> {
            match self.docs.get(url) {
                Some(body) => Ok(sync::Fetched {
                    final_url: url.to_string(),
                    content_type: Some("application/json".to_string()),
                    body: body.clone(),
                }),
                // What a dead source looks like from here: sources 502 scrapers,
                // disappear, and paywall.
                None => anyhow::bail!("no fixture for {url}"),
            }
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

    /// Collects raw in memory: the sink is not what these tests are about, and
    /// keeping it out of the database leaves `derive` reading a corpus the test
    /// controls.
    #[derive(Default)]
    struct MemorySink;

    impl sync::Sink for MemorySink {
        async fn store_raw(&self, _: &Ingested, _: Option<&str>, _: i64) -> anyhow::Result<()> {
            Ok(())
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

    /// The status and `finished_at` of the newest `ingest` run — what the trigger
    /// left behind.
    async fn last_ingest_run(conn: &Connection) -> (String, Option<i64>) {
        let mut rows = conn
            .query(
                "SELECT status, finished_at FROM runs WHERE kind = 'ingest'
                 ORDER BY id DESC LIMIT 1",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (
            row.get::<String>(0).unwrap(),
            row.get::<Option<i64>>(1).unwrap(),
        )
    }

    /// A trigger that did the whole job still records `completed` — the fix must not
    /// have swapped one constant answer for another.
    #[tokio::test]
    async fn a_whole_trigger_still_records_completed() {
        let conn = conn().await;
        let fetcher = fetcher_with(&[("fix://soup", "Soup"), ("fix://stew", "Stew")]);

        let report = pipeline(&conn, &[FIXTURE], &fetcher, &MemorySink).await;

        assert_eq!(report.sync.fetched, 2, "both catalog urls fetched");
        assert!(report.sync.failures.is_empty());
        assert_eq!(last_ingest_run(&conn).await.0, runs::COMPLETED);
    }

    /// The heart of #174: a derive that blows up must not be recorded as a completed
    /// run. Dropping `raw_imports` is the shape a real failure takes — `derive`'s read
    /// of it comes back a database error, exactly as it did when Turso truncated every
    /// streamed response for seven hours (#146).
    #[tokio::test]
    async fn a_failed_derive_is_not_recorded_as_completed() {
        let conn = conn().await;
        conn.execute("DROP TABLE raw_imports", ()).await.unwrap();
        let fetcher = fetcher_with(&[("fix://soup", "Soup"), ("fix://stew", "Stew")]);

        let report = pipeline(&conn, &[FIXTURE], &fetcher, &MemorySink).await;

        let (status, finished_at) = last_ingest_run(&conn).await;
        assert_eq!(
            status,
            runs::FAILED,
            "a derive that failed is not completed"
        );
        assert!(
            finished_at.is_some(),
            "and it is closed, not abandoned — an open row means the process died"
        );
        // The degrade holds: the trigger answers with a report rather than 500-ing a
        // scheduled job, and the sync half of it is intact.
        assert_eq!(report.derive, derive::Report::default());
        assert_eq!(report.sync.fetched, 2);
    }

    /// A source that would not answer is not a failed run — sources 502 scrapers as a
    /// matter of routine, and calling that `failed` would pin the column to one value
    /// all over again. It is not a whole run either, and `partial` is that.
    #[tokio::test]
    async fn a_dead_source_records_partial() {
        let conn = conn().await;
        // `fix://stew` is absent from the fetcher — that source is down this run.
        let fetcher = fetcher_with(&[("fix://soup", "Soup")]);

        let report = pipeline(&conn, &[FIXTURE], &fetcher, &MemorySink).await;

        assert_eq!(report.sync.failures.len(), 1, "one url could not be had");
        assert_eq!(report.sync.failures[0].url, "fix://stew");
        assert_eq!(last_ingest_run(&conn).await.0, runs::PARTIAL);
    }

    /// A trigger that fetched *nothing* is the case #174 opens with: today it read
    /// `completed`, indistinguishable from one that did the whole job. It is `partial`
    /// — every URL is named in `failures`, and the count is the report's business, not
    /// the status's.
    #[tokio::test]
    async fn a_trigger_that_fetched_nothing_is_not_completed() {
        let conn = conn().await;
        let report = pipeline(&conn, &[FIXTURE], &fetcher_with(&[]), &MemorySink).await;

        assert_eq!(report.sync.fetched, 0);
        assert_eq!(report.sync.failures.len(), 2, "every source was down");
        assert_eq!(last_ingest_run(&conn).await.0, runs::PARTIAL);
    }

    /// A run is as bad as its worst stage: a derive that failed is not redeemed by the
    /// sync beside it, and a partial sync does not soften it either.
    #[tokio::test]
    async fn a_failed_derive_outranks_a_partial_sync() {
        let conn = conn().await;
        conn.execute("DROP TABLE raw_imports", ()).await.unwrap();
        let report = pipeline(&conn, &[FIXTURE], &fetcher_with(&[]), &MemorySink).await;

        assert_eq!(report.sync.failures.len(), 2, "the sync was partial too");
        assert_eq!(last_ingest_run(&conn).await.0, runs::FAILED);
    }
}
