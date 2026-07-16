//! `POST /api/ingest` — pull every source's catalog into the corpus.
//!
//! This used to take a client-supplied URL and ingest that one document —
//! "ingest is what a search does". It no longer does: the client hits a **trigger
//! with no target**, and the server dispatches to every adapter's catalog itself,
//! fetches, normalizes, and stores. There is no query; search is gone (#49).
//!
//! The whole engine lives in [`crate::sync`], behind [`sync::Fetcher`] and
//! [`sync::Sink`] so it can be tested against a fixture adapter. Here we just wire
//! the production effects — SSRF-guarded HTTP and the Turso store — and run it.
//!
//! Still session-gated: it is under the auth-required routes. A sync spends
//! fetches against sources, so who may trigger it is worth revisiting (#49), but
//! "any signed-in member" is the current answer.

use axum::{extract::State, Json};
use recipe_core::adapters;

use crate::{sync, AppState};

/// `POST /api/ingest` — trigger a server-driven corpus sync; report what it did.
pub async fn ingest(State(state): State<AppState>) -> Json<sync::SyncReport> {
    let fetcher = sync::ProxyFetcher { http: &state.http };
    let sink = sync::TursoSink { conn: &state.db };
    Json(sync::sync(adapters::ADAPTERS, &fetcher, &sink).await)
}
