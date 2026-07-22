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
use recipe_core::adapters;
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

    // One run for the whole trigger, so every write it makes is ordered against a
    // concurrent CLI `enrich`/`derive` (#11 write-path hardening). Best-effort: if
    // the run row can't be opened, stamp 0 — superseded by any real run — rather
    // than 500 a scheduled trigger.
    let run_id = runs::begin(&db, "ingest").await.unwrap_or_else(|e| {
        tracing::warn!("could not open a run, stamping 0: {e}");
        0
    });

    let fetcher = sync::ProxyFetcher { http: &state.http };
    let sink = sync::TursoSink { conn: &db };
    let sync = sync::sync(adapters::ADAPTERS, &fetcher, &sink, run_id).await;

    // Derive so `recipes` reflects the raw just synced (and reattaches whatever
    // readings the enrich worker has already stored). Best-effort: a failed derive
    // leaves the previous `recipes` in place rather than 500-ing a scheduled
    // trigger.
    let derive = derive::derive(&db, None, run_id).await.unwrap_or_else(|e| {
        tracing::warn!("derive step failed, leaving recipes as-is: {e}");
        derive::Report::default()
    });

    // Close the run. A run left open (this failing, or the process dying) is the
    // "died mid-flight" signal the runs table exists to surface.
    if let Err(e) = runs::finish(&db, run_id, runs::COMPLETED).await {
        tracing::warn!("could not close run {run_id}: {e}");
    }

    Json(IngestReport { sync, derive })
}
