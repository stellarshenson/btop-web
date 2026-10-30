//! Request errors. The full error is logged; the response body is sanitized.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("template rendering failed: {0}")]
    Template(#[from] askama::Error),
    /// ACC-NET-143-AGENT-MODES: a page or stream requested for an agent the
    /// client does not know.
    #[error("not found")]
    NotFound,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Template(error) => {
                tracing::error!(error = %error, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
            Self::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
        }
    }
}
