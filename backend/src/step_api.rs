//! The step-reading work queue over HTTP (#74/#75/#76): the two machine-gated
//! endpoints a worker uses to read a recipe's method into a [`StructuredStep`] DAG,
//! and the thin client the `recipe-backend steps pull|push` commands call.
//!
//! The shape mirrors [`crate::enrich_api`] exactly — reading messy prose into
//! structure is an LLM job that runs **off** this service, so the worker pulls work
//! and pushes results through the app's validating front door, holding only the
//! app's URL and the machine key, never a database token.
//!
//! - [`pending`] — `GET /api/enrich/steps/pending?limit=N`: recipes with a method but
//!   no step reading yet, plus the ingredients (with any preparation) for prep
//!   extraction (#76).
//! - [`results`] — `POST /api/enrich/steps/results`: the worker's step DAGs. The
//!   server validates each graph and stores + re-derives via [`steps::submit`].
//!
//! Both are gated by `INGEST_API_KEY`, the same machine gate as ingest and the
//! ingredient enrichment — the worker authenticates as infrastructure.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{error::AppError, steps, AppState};

/// Same bounds as the ingredient queue: a predictable pull payload; the worker loops.
const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PendingParams {
    limit: Option<usize>,
}

/// `GET /api/enrich/steps/pending?limit=N` — recipes with no step reading yet.
pub async fn pending(
    State(state): State<AppState>,
    Query(params): Query<PendingParams>,
) -> Result<Json<Vec<steps::PendingStepRecipe>>, AppError> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let recipes = steps::pending(&state.db()?, limit)
        .await
        .map_err(|e| AppError::Internal(format!("steps pending failed: {e}")))?;
    Ok(Json(recipes))
}

/// The body of a `POST /api/enrich/steps/results`: the worker's step DAGs for one
/// batch, plus the model that produced them. `readings` (not `steps`) names the batch
/// so the shape matches the ingredient endpoint — each entry carries its own `steps`.
#[derive(Debug, Deserialize)]
pub struct ResultsRequest {
    model: String,
    readings: Vec<steps::SubmittedSteps>,
}

/// `POST /api/enrich/steps/results` — store a worker's step readings and re-derive.
pub async fn results(
    State(state): State<AppState>,
    Json(req): Json<ResultsRequest>,
) -> Result<Json<steps::SubmitReport>, AppError> {
    if req.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "model is required — it is the reading's provenance".into(),
        ));
    }
    let report = steps::submit(&state.db()?, req.readings, req.model.trim())
        .await
        .map_err(|e| AppError::Internal(format!("steps results failed: {e}")))?;
    Ok(Json(report))
}

/// The worker side: the thin HTTP client the `steps pull|push` CLI commands call.
/// Reuses the ingredient client's plumbing (target, timeouts, the input coercion) —
/// both talk to the same app with the same key; only the paths differ.
pub mod client {
    use crate::enrich_api::client::{http, normalize_readings, require_model, Target};
    use serde_json::{json, Value};

    /// GET the pending step recipes and return the response body. Shared core of the
    /// `pull` CLI command and the `step_pull` MCP tool.
    pub async fn pull_pending(limit: Option<usize>) -> anyhow::Result<String> {
        let target = Target::from_env()?;
        let mut url = format!("{}/api/enrich/steps/pending", target.base_url);
        if let Some(n) = limit {
            url.push_str(&format!("?limit={n}"));
        }
        let resp = http()?.get(url).bearer_auth(&target.api_key).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("steps pending request failed ({status}): {body}");
        }
        Ok(body)
    }

    /// `recipe-backend steps pull [--limit N]` — the CLI form: print the pending JSON.
    pub async fn pull(limit: Option<usize>) -> anyhow::Result<()> {
        println!("{}", pull_pending(limit).await?);
        Ok(())
    }

    /// POST a batch of step readings and return the summary. Stamps the model from
    /// `ENRICH_MODEL`. `readings` is the JSON array of `{source, id, steps}`, supplied
    /// on stdin (CLI) or as a typed tool argument (MCP).
    pub async fn push_readings(readings: Value) -> anyhow::Result<String> {
        let readings = normalize_readings(readings)?;
        let target = Target::from_env()?;
        let model = require_model(std::env::var("ENRICH_MODEL").ok())?;
        let body = json!({ "model": model, "readings": readings });

        let resp = http()?
            .post(format!("{}/api/enrich/steps/results", target.base_url))
            .bearer_auth(&target.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("steps results request failed ({status}): {text}");
        }
        Ok(text)
    }

    /// `… | recipe-backend steps push` — the CLI form: read the skill's step JSON from
    /// stdin, forward, print the summary.
    pub async fn push() -> anyhow::Result<()> {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        println!("{}", push_readings(Value::String(input)).await?);
        Ok(())
    }
}
