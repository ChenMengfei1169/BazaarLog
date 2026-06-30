// Unified application error type mapped to HTTP responses.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("resource not found")]
    NotFound,
    #[error("unauthorized: class password required or invalid")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Database(_) | AppError::Internal(_) => {
                // Use Debug formatting so the full SQLX::Error source chain
                // (including the underlying driver message) lands in the log.
                tracing::error!(error = ?self, "request failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // Never leak internal details to the client; the message is generic for
        // server-class errors and logged above with full context.
        let message = match &self {
            AppError::BadRequest(msg) => msg.clone(),
            AppError::NotFound | AppError::Unauthorized => self.to_string(),
            AppError::Database(_) | AppError::Internal(_) => "internal server error".into(),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
