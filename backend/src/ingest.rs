//! `POST /api/ingest` — pull every source's catalog into the corpus, enrich the
//! new lines, and derive.
//!
//! This used to take a client-supplied URL and ingest that one document —
//! "ingest is what a search does". It no longer does: the client hits a **trigger
//! with no target**, and the server dispatches to every adapter's catalog itself,
//! fetches, normalizes, and stores. There is no query; search is gone (#49).
//!
//! One trigger runs the whole pipeline (#11), under one `run_id` so its writes are
//! ordered against any concurrent CLI run: `sync` fetches and writes `raw_imports`,
//! `enrich` reads each recipe's lines into `ingredient_structures` (the only
//! networked-LLM step; skipped when no endpoint is configured), and `derive`
//! rebuilds `recipes` from raw + readings. Each stage writes one table — the write
//! path is decoupled. The sync engine lives in [`crate::sync`], behind
//! [`sync::Fetcher`]/[`sync::Sink`] so it can be tested against a fixture.
//!
//! **Machine-gated, not session-gated**: `Authorization: Bearer <INGEST_API_KEY>`
//! (see [`crate::auth::require_api_key`]). A browser session does not authorize
//! this endpoint — the client has no access to ingestion at all, which is the
//! point of #49. A schedule holds the key; nobody presses a button.

use axum::{extract::State, Json};
use recipe_core::adapters;
use serde::Serialize;

use crate::{derive, enrich, runs, sync, AppState};

/// What the whole ingest pipeline did — each stage's report, so the scheduled job
/// can log fetch/enrich/derive counts from one response.
#[derive(Serialize)]
pub struct IngestReport {
    sync: sync::SyncReport,
    /// `None` when no extractor is configured (`LLM_BASE_URL`/`LLM_MODEL` unset) or
    /// the enrich step errored — the corpus still syncs and derives, just without
    /// new structured readings. Degrade-not-die.
    enrich: Option<enrich::EnrichReport>,
    derive: derive::Report,
}

/// `POST /api/ingest` — trigger a server-driven corpus sync + enrich + derive.
pub async fn ingest(State(state): State<AppState>) -> Json<IngestReport> {
    // One run for the whole pipeline, so every write it makes is ordered against a
    // concurrent CLI `enrich`/`derive` (#11 write-path hardening). Best-effort: if
    // the run row can't be opened, stamp 0 — superseded by any real run — rather
    // than 500 a scheduled trigger.
    let run_id = runs::begin(&state.db, "ingest").await.unwrap_or_else(|e| {
        tracing::warn!("could not open a run, stamping 0: {e}");
        0
    });

    let fetcher = sync::ProxyFetcher { http: &state.http };
    let sink = sync::TursoSink { conn: &state.db };
    let sync = sync::sync(adapters::ADAPTERS, &fetcher, &sink, run_id).await;

    // Enrich the newly-seen lines, if a key is configured. Best-effort: an enrich
    // error (e.g. the API is down) must not fail the trigger — derive still runs
    // and reattaches whatever is already cached.
    let enrich = match state.extractor.as_ref() {
        // Routine, not a refresh: only recipes with no reading yet. A model-driven
        // re-snapshot is the deliberate `enrich --refresh`, never the daily path.
        Some(extractor) => match enrich::enrich(&state.db, extractor, false, run_id).await {
            Ok(report) => Some(report),
            Err(e) => {
                tracing::warn!("enrich step failed, continuing: {e}");
                None
            }
        },
        None => None,
    };

    // Derive so `recipes` picks up the readings just stored. Also best-effort: a
    // failed derive leaves the previous `recipes` in place rather than 500-ing a
    // scheduled trigger.
    let derive = derive::derive(&state.db, None, run_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("derive step failed, leaving recipes as-is: {e}");
            derive::Report::default()
        });

    // Close the run. A run left open (this failing, or the process dying) is the
    // "died mid-flight" signal the runs table exists to surface.
    if let Err(e) = runs::finish(&state.db, run_id, runs::COMPLETED).await {
        tracing::warn!("could not close run {run_id}: {e}");
    }

    Json(IngestReport {
        sync,
        enrich,
        derive,
    })
}
