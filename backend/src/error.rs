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
    /// No live session. Auth is mandatory (#25), so this is the default answer
    /// to an anonymous caller.
    #[error("{0}")]
    Unauthorized(String),
    /// The target resolves to a non-public address (SSRF guard).
    #[error("blocked: target address is not permitted")]
    Blocked,
    #[error("upstream fetch failed: {0}")]
    Upstream(String),
    #[error("internal error")]
    Internal(String),
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
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Blocked => StatusCode::FORBIDDEN,
            AppError::Upstream(e) => {
                tracing::warn!("upstream error: {e}");
                StatusCode::BAD_GATEWAY
            }
            AppError::Internal(e) => {
                tracing::error!("internal error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
