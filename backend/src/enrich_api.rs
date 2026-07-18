//! The enrichment work queue over HTTP (#59): the two machine-gated endpoints a
//! worker uses, and the thin client the `recipe-backend enrich pull|push` commands
//! call to reach them.
//!
//! Enrichment reads a recipe's raw ingredient lines into structure — an LLM job
//! that runs **off** this service. The worker (the `recipes-enrich` skill) does the
//! reading; the app owns the corpus. So the worker never touches the database: it
//! pulls work and pushes readings through the app's front door, and the app
//! validates every submission before it writes a row. An LLM producing corpus rows
//! directly is exactly what this refuses — the model's output is untrusted input
//! that crosses a validating boundary, like any other request.
//!
//! - [`pending`] — `GET /api/enrich/pending?limit=N`: recipes with no reading yet,
//!   and their ingredient lines. The worker's "what needs doing".
//! - [`results`] — `POST /api/enrich/results`: the worker's readings. The server
//!   validates each (the recipe still exists, the reading count matches its
//!   *current* ingredient list) and stores + re-derives via [`enrich::submit`].
//!
//! Both are gated by `INGEST_API_KEY` (the machine gate, [`crate::auth`]) — the
//! worker authenticates as infrastructure, exactly like the ingest trigger, and
//! holds only that key and the app's URL, never a database token.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{enrich, error::AppError, AppState};

/// How many recipes `pending` returns when the caller names no limit, and the most
/// it will ever return. A bound keeps one pull's payload — and the worker's next
/// extraction batch — a predictable size; the worker loops until the queue drains.
const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PendingParams {
    limit: Option<usize>,
}

/// `GET /api/enrich/pending?limit=N` — recipes with no stored reading yet.
pub async fn pending(
    State(state): State<AppState>,
    Query(params): Query<PendingParams>,
) -> Result<Json<Vec<enrich::PendingRecipe>>, AppError> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let recipes = enrich::pending(&state.db, limit)
        .await
        .map_err(|e| AppError::Internal(format!("pending failed: {e}")))?;
    Ok(Json(recipes))
}

/// The body of a `POST /api/enrich/results`: the worker's readings for one batch,
/// plus the model that produced them (stamped once per batch, not per recipe).
#[derive(Debug, Deserialize)]
pub struct ResultsRequest {
    /// Provenance for every reading in this batch — the model the worker used.
    /// Required and non-blank: a missing model would record a reading with no
    /// traceable source, which for a non-deterministic, drifting input is the one
    /// thing we must not lose.
    model: String,
    readings: Vec<enrich::SubmittedReadings>,
}

/// `POST /api/enrich/results` — store a worker's readings and re-derive.
///
/// The model is validated here, at the boundary, rather than trusted from the
/// client: a blank one is a bad request, never a silent placeholder.
pub async fn results(
    State(state): State<AppState>,
    Json(req): Json<ResultsRequest>,
) -> Result<Json<enrich::SubmitReport>, AppError> {
    if req.model.trim().is_empty() {
        return Err(AppError::BadRequest(
            "model is required — it is the readings' provenance".into(),
        ));
    }
    let report = enrich::submit(&state.db, req.readings, req.model.trim())
        .await
        .map_err(|e| AppError::Internal(format!("results failed: {e}")))?;
    Ok(Json(report))
}

/// The worker side: the thin HTTP client the `enrich pull|push` CLI commands call.
///
/// This is the *only* thing the enrich skill drives, and it does no more than move
/// JSON between stdin/stdout and the app's two endpoints. It holds no database
/// connection — just the app's URL and the machine API key — so the model behind it
/// can never reach the corpus except through the app's validating front door.
pub mod client {
    use serde_json::{json, Value};

    /// Read a required worker env var, or explain what is missing. Pure over its
    /// input so it is tested without mutating the process environment.
    fn require(name: &str, value: Option<String>) -> anyhow::Result<String> {
        match value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
            Some(v) => Ok(v),
            None => anyhow::bail!("{name} is required (set it in the worker's environment)"),
        }
    }

    /// The model the readings should be stamped with. **Required, no default**: a
    /// placeholder like `"claude"` would record a vendor name as though it were the
    /// model, which is worse than refusing — provenance you can't trust is the point
    /// of recording it (CodeRabbit, PR #60).
    pub fn require_model(value: Option<String>) -> anyhow::Result<String> {
        require("ENRICH_MODEL", value)
    }

    /// Where the app lives + the machine key, from the worker's environment.
    struct Target {
        base_url: String,
        api_key: String,
    }

    impl Target {
        fn from_env() -> anyhow::Result<Self> {
            let base_url = require("RECIPES_API_URL", std::env::var("RECIPES_API_URL").ok())?;
            let api_key = require("INGEST_API_KEY", std::env::var("INGEST_API_KEY").ok())?;
            Ok(Self {
                base_url: base_url.trim_end_matches('/').to_owned(),
                api_key,
            })
        }
    }

    /// GET the pending recipes from the app and return the response body (the JSON
    /// array). The shared core of the `pull` CLI command and the `enrich_pull` MCP
    /// tool — neither prints; the caller decides what to do with the body.
    pub async fn pull_pending(limit: Option<usize>) -> anyhow::Result<String> {
        let target = Target::from_env()?;
        let mut url = format!("{}/api/enrich/pending", target.base_url);
        if let Some(n) = limit {
            url.push_str(&format!("?limit={n}"));
        }
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(&target.api_key)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("pending request failed ({status}): {body}");
        }
        Ok(body)
    }

    /// `recipe-backend enrich pull [--limit N]` — the CLI form: print what
    /// [`pull_pending`] returns to stdout, for a Bash-driven skill.
    pub async fn pull(limit: Option<usize>) -> anyhow::Result<()> {
        println!("{}", pull_pending(limit).await?);
        Ok(())
    }

    /// POST a batch of readings to the app and return the response body (the
    /// accepted/derived/rejected summary). Stamps the model from `ENRICH_MODEL`. The
    /// shared core of the `push` CLI command and the `enrich_push` MCP tool —
    /// `readings` is the JSON array of `{source, id, readings}`, supplied on stdin
    /// (CLI) or as a typed tool argument (MCP). The app validates and writes; this
    /// only forwards.
    pub async fn push_readings(readings: Value) -> anyhow::Result<String> {
        let target = Target::from_env()?;
        let model = require_model(std::env::var("ENRICH_MODEL").ok())?;
        let body = json!({ "model": model, "readings": readings });

        let resp = reqwest::Client::new()
            .post(format!("{}/api/enrich/results", target.base_url))
            .bearer_auth(&target.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("results request failed ({status}): {text}");
        }
        Ok(text)
    }

    /// `… | recipe-backend enrich push` — the CLI form: read the skill's readings
    /// JSON from stdin, forward via [`push_readings`], print the summary to stdout.
    pub async fn push() -> anyhow::Result<()> {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let readings = normalize_readings(&input)?;
        println!("{}", push_readings(readings).await?);
        Ok(())
    }

    /// The skill emits either a bare array of `{source, id, readings}` or a
    /// `{ "readings": [...] }` object wrapping it — accept both and return the array.
    /// Pure, so the one-wrapper tolerance is tested directly.
    fn normalize_readings(input: &str) -> anyhow::Result<Value> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(json!([]));
        }
        match serde_json::from_str::<Value>(trimmed)? {
            arr @ Value::Array(_) => Ok(arr),
            Value::Object(mut map) => map
                .remove("readings")
                .filter(Value::is_array)
                .ok_or_else(|| anyhow::anyhow!("push input object has no `readings` array")),
            _ => anyhow::bail!(
                "push input must be a JSON array or an object with a `readings` array"
            ),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A required var must be present and non-blank; whitespace-only counts as
        /// missing, and a value is trimmed. Pure — no `set_var`.
        #[test]
        fn require_rejects_missing_and_blank() {
            assert!(require("X", None).is_err());
            assert!(require("X", Some("".into())).is_err());
            assert!(require("X", Some("   ".into())).is_err());
            assert_eq!(require("X", Some("  v  ".into())).unwrap(), "v");
        }

        /// The model is required with no default — `"claude"` must never be
        /// substituted for a missing one.
        #[test]
        fn require_model_has_no_default() {
            assert!(require_model(None).is_err());
            assert!(require_model(Some("  ".into())).is_err());
            assert_eq!(
                require_model(Some("claude-opus-4-8".into())).unwrap(),
                "claude-opus-4-8"
            );
        }

        /// The push input accepts a bare array or a `{readings:[...]}` wrapper, and
        /// rejects anything else.
        #[test]
        fn normalize_readings_accepts_array_or_wrapper() {
            assert_eq!(normalize_readings("").unwrap(), json!([]));
            assert_eq!(
                normalize_readings(r#"[{"source":"s","id":"1","readings":[]}]"#).unwrap(),
                json!([{"source":"s","id":"1","readings":[]}])
            );
            assert_eq!(
                normalize_readings(r#"{"readings":[{"source":"s","id":"1","readings":[]}]}"#)
                    .unwrap(),
                json!([{"source":"s","id":"1","readings":[]}])
            );
            // An object without a `readings` array, or a scalar, is an error.
            assert!(normalize_readings(r#"{"nope":1}"#).is_err());
            assert!(normalize_readings("42").is_err());
            assert!(normalize_readings("not json").is_err());
        }
    }
}
