//! Application error type and its HTTP representation.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    /// The target resolves to a non-public address (SSRF guard).
    #[error("blocked: target address is not permitted")]
    Blocked,
    #[error("upstream fetch failed: {0}")]
    Upstream(String),
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Upstream(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Blocked => StatusCode::FORBIDDEN,
            AppError::Upstream(e) => {
                tracing::warn!("upstream error: {e}");
                StatusCode::BAD_GATEWAY
            }
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
