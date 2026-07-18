//! `POST /api/ingest` — pull every source's catalog into the corpus, enrich the
//! new lines, and derive.
//!
//! This used to take a client-supplied URL and ingest that one document —
//! "ingest is what a search does". It no longer does: the client hits a **trigger
//! with no target**, and the server dispatches to every adapter's catalog itself,
//! fetches, normalizes, and stores. There is no query; search is gone (#49).
//!
//! One trigger runs the whole pipeline (#11): `sync` fetches and stores both
//! halves, `enrich` reads the new lines into `ingredient_structured` (the only
//! networked-LLM step; skipped when no key is configured), and `derive` rebuilds
//! `recipes` so it picks up the readings. The sync engine lives in [`crate::sync`],
//! behind [`sync::Fetcher`]/[`sync::Sink`] so it can be tested against a fixture.
//!
//! **Machine-gated, not session-gated**: `Authorization: Bearer <INGEST_API_KEY>`
//! (see [`crate::auth::require_api_key`]). A browser session does not authorize
//! this endpoint — the client has no access to ingestion at all, which is the
//! point of #49. A schedule holds the key; nobody presses a button.

use axum::{extract::State, Json};
use recipe_core::adapters;
use serde::Serialize;

use crate::{derive, enrich, sync, AppState};

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
    let fetcher = sync::ProxyFetcher { http: &state.http };
    let sink = sync::TursoSink { conn: &state.db };
    let sync = sync::sync(adapters::ADAPTERS, &fetcher, &sink).await;

    // Enrich the newly-seen lines, if a key is configured. Best-effort: an enrich
    // error (e.g. the API is down) must not fail the trigger — derive still runs
    // and reattaches whatever is already cached.
    let enrich = match state.extractor.as_ref() {
        Some(extractor) => match enrich::enrich(&state.db, extractor).await {
            Ok(report) => Some(report),
            Err(e) => {
                tracing::warn!("enrich step failed, continuing: {e}");
                None
            }
        },
        None => None,
    };

    // Derive so `recipes` picks up the readings just cached. Also best-effort: a
    // failed derive leaves the previous `recipes` in place rather than 500-ing a
    // scheduled trigger.
    let derive = derive::derive(&state.db, None).await.unwrap_or_else(|e| {
        tracing::warn!("derive step failed, leaving recipes as-is: {e}");
        derive::Report::default()
    });

    Json(IngestReport {
        sync,
        enrich,
        derive,
    })
}
