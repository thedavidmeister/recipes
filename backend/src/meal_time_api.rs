//! The meal-time work queue over HTTP (#191): the machine-gated endpoints a worker uses
//! to read when each dish is eaten.
//!
//! The shape mirrors [`crate::nutrition_api`] exactly — the model runs **off** this
//! service (#59), so the worker pulls work and pushes readings through the app's
//! validating front door, holding the machine key and never a database token.
//!
//! - [`pending`] — `GET /api/enrich/meal-times/pending?limit=N`
//! - [`results`] — `POST /api/enrich/meal-times/results`
//!
//! There is no third, person-facing endpoint here. Nothing a person does needs this
//! reading in its raw form: the app serves the *derived* answer by narrowing the walk
//! (#191/#184), and a plan's own meal comes from the lobby, not from here.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{error::AppError, meal_times, AppState};

const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PendingParams {
    limit: Option<usize>,
}

/// `GET /api/enrich/meal-times/pending?limit=N` — recipes with no meal-time reading.
pub async fn pending(
    State(state): State<AppState>,
    Query(params): Query<PendingParams>,
) -> Result<Json<Vec<meal_times::PendingMealTimeRecipe>>, AppError> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let recipes = state
        .with_db(move |conn| async move { meal_times::pending(&conn, limit).await })
        .await
        .map_err(|e| AppError::Internal(format!("meal-time pending failed: {e:#}")))?;
    Ok(Json(recipes))
}

#[derive(Debug, Deserialize)]
pub struct ResultsRequest {
    model: String,
    readings: Vec<meal_times::SubmittedMealTimes>,
}

/// `POST /api/enrich/meal-times/results` — store readings and re-derive.
pub async fn results(
    State(state): State<AppState>,
    Json(req): Json<ResultsRequest>,
) -> Result<Json<meal_times::SubmitReport>, AppError> {
    if req.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "model is required — it is the reading's provenance".into(),
        ));
    }
    // Safe to re-run on a transient failure: every write in `submit` is a
    // `run_id`-guarded upsert (#11, #130).
    let readings = &req.readings;
    let model = req.model.trim();
    let report = state
        .with_db(
            move |conn| async move { meal_times::submit(&conn, readings.clone(), model).await },
        )
        .await
        .map_err(|e| AppError::Internal(format!("meal-time results failed: {e:#}")))?;
    Ok(Json(report))
}

/// The worker side: the thin HTTP client the `meal-times pull|push` CLI commands call,
/// reusing the ingredient client's plumbing (target, timeouts, input coercion) — the
/// same app, the same key, only the paths differ.
pub mod client {
    use crate::enrich_api::client::{http, normalize_readings, require_model, Target};
    use serde_json::{json, Value};

    /// GET the recipes still needing a meal-time reading.
    pub async fn pull_pending(limit: Option<usize>) -> anyhow::Result<String> {
        let target = Target::from_env()?;
        let mut url = format!("{}/api/enrich/meal-times/pending", target.base_url);
        if let Some(n) = limit {
            url.push_str(&format!("?limit={n}"));
        }
        let resp = http()?.get(url).bearer_auth(&target.api_key).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("meal-time pending request failed ({status}): {body}");
        }
        Ok(body)
    }

    /// `recipe-backend meal-times pull [--limit N]`.
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
            .post(format!("{}/api/enrich/meal-times/results", target.base_url))
            .bearer_auth(&target.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("meal-time results request failed ({status}): {text}");
        }
        Ok(text)
    }

    /// `… | recipe-backend meal-times push`.
    pub async fn push() -> anyhow::Result<()> {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        println!("{}", push_readings(Value::String(input)).await?);
        Ok(())
    }
}
