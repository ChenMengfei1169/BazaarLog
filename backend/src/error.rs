// Unified application error type mapped to HTTP responses. Every handler
// returns AppResult, so the single IntoResponse implementation below decides
// the status code and body for every failure path.
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
    #[error("forbidden: insufficient permission")]
    Forbidden,
    #[error("too many requests")]
    RateLimited,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("conflict: {0}")]
    Conflict(String),
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
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Database(_) | AppError::Internal(_) => {
                // Debug formatting keeps the full sqlx error source chain,
                // including the underlying driver message, in the log.
                tracing::error!(error = ?self, "request failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // Never leak internal details to the client: the message is generic for
        // server-class errors and the full context is logged above.
        let message = match &self {
            AppError::BadRequest(message) | AppError::Conflict(message) => message.clone(),
            AppError::NotFound
            | AppError::Unauthorized
            | AppError::Forbidden
            | AppError::RateLimited => self.to_string(),
            AppError::Database(_) | AppError::Internal(_) => "internal server error".into(),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Shorthand for a handler result that carries an AppError.
pub type AppResult<T> = Result<T, AppError>;
