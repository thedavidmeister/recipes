//! The equipment-reading work queue over HTTP (#81): the machine-gated endpoints a
//! worker uses to read what a recipe requires, plus the vocabulary a kitchen picks
//! from.
//!
//! The shape mirrors [`crate::step_api`] exactly — the model runs **off** this service
//! (#59), so the worker pulls work and pushes readings through the app's validating
//! front door, holding the machine key and never a database token.
//!
//! - [`pending`] — `GET /api/enrich/equipment/pending?limit=N`
//! - [`results`] — `POST /api/enrich/equipment/results`
//! - [`vocabulary`] — `GET /api/equipment` : every distinct item the corpus knows.
//!
//! The first two are gated by `INGEST_API_KEY` like every other worker endpoint. The
//! third is **session-gated**, not machine-gated: it is a person's picker, and it says
//! nothing a person could not already read out of the corpus.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{equipment, error::AppError, AppState};

const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PendingParams {
    limit: Option<usize>,
}

/// `GET /api/enrich/equipment/pending?limit=N` — recipes with no equipment reading.
pub async fn pending(
    State(state): State<AppState>,
    Query(params): Query<PendingParams>,
) -> Result<Json<Vec<equipment::PendingEquipmentRecipe>>, AppError> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let recipes = equipment::pending(&state.db()?, limit)
        .await
        .map_err(|e| AppError::Internal(format!("equipment pending failed: {e}")))?;
    Ok(Json(recipes))
}

#[derive(Debug, Deserialize)]
pub struct ResultsRequest {
    model: String,
    readings: Vec<equipment::SubmittedEquipment>,
}

/// `POST /api/enrich/equipment/results` — store readings and re-derive.
pub async fn results(
    State(state): State<AppState>,
    Json(req): Json<ResultsRequest>,
) -> Result<Json<equipment::SubmitReport>, AppError> {
    if req.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "model is required — it is the reading's provenance".into(),
        ));
    }
    let report = equipment::submit(&state.db()?, req.readings, req.model.trim())
        .await
        .map_err(|e| AppError::Internal(format!("equipment results failed: {e}")))?;
    Ok(Json(report))
}

/// The worker side: the thin HTTP client the `equipment pull|push` CLI commands call,
/// reusing the ingredient client's plumbing (target, timeouts, input coercion) — the
/// same app, the same key, only the paths differ.
pub mod client {
    use crate::enrich_api::client::{http, normalize_readings, require_model, Target};
    use serde_json::{json, Value};

    /// GET the recipes still needing an equipment reading.
    pub async fn pull_pending(limit: Option<usize>) -> anyhow::Result<String> {
        let target = Target::from_env()?;
        let mut url = format!("{}/api/enrich/equipment/pending", target.base_url);
        if let Some(n) = limit {
            url.push_str(&format!("?limit={n}"));
        }
        let resp = http()?.get(url).bearer_auth(&target.api_key).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("equipment pending request failed ({status}): {body}");
        }
        Ok(body)
    }

    /// `recipe-backend equipment pull [--limit N]`.
    pub async fn pull(limit: Option<usize>) -> anyhow::Result<()> {
        println!("{}", pull_pending(limit).await?);
        Ok(())
    }

    /// POST a batch of readings, stamping the model from `ENRICH_MODEL`.
    pub async fn push_readings(readings: Value) -> anyhow::Result<String> {
        let readings = normalize_readings(readings)?;
        let target = Target::from_env()?;
        let model = require_model(std::env::var("ENRICH_MODEL").ok())?;
        let body = json!({ "model": model, "readings": readings });

        let resp = http()?
            .post(format!("{}/api/enrich/equipment/results", target.base_url))
            .bearer_auth(&target.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("equipment results request failed ({status}): {text}");
        }
        Ok(text)
    }

    /// `… | recipe-backend equipment push`.
    pub async fn push() -> anyhow::Result<()> {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        println!("{}", push_readings(Value::String(input)).await?);
        Ok(())
    }
}

/// `GET /api/equipment` — every distinct item the corpus knows about.
///
/// This is what a kitchen picks from, and the whole of what it may pick from (#81): a
/// kitchen never invents equipment, because an item no recipe mentions could not change
/// what you are able to cook.
pub async fn vocabulary(
    State(state): State<AppState>,
    axum::Extension(_user): axum::Extension<crate::auth::CurrentUser>,
) -> Result<Json<Vec<String>>, AppError> {
    let items = equipment::vocabulary(&state.db()?)
        .await
        .map_err(|e| AppError::Internal(format!("equipment vocabulary failed: {e}")))?;
    Ok(Json(items))
}

/// `GET /api/pantry` — every ingredient the corpus cooks with, the list a kitchen's
/// pantry picks from. Session-gated for the same reason as the equipment list: a
/// person's picker, revealing nothing they could not read out of the corpus.
pub async fn pantry_vocabulary(
    State(state): State<AppState>,
    axum::Extension(_user): axum::Extension<crate::auth::CurrentUser>,
) -> Result<Json<Vec<String>>, AppError> {
    let items = crate::enrich::vocabulary(&state.db()?)
        .await
        .map_err(|e| AppError::Internal(format!("pantry vocabulary failed: {e}")))?;
    Ok(Json(items))
}
