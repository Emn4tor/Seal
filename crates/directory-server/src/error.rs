use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("unknown signer")]
    UnknownSigner,
    #[error("replayed or stale request")]
    Replay,
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(err, _) = &e
            && err.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return AppError::Conflict("record already exists".to_string());
        }
        AppError::Internal(anyhow::Error::new(e))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::InvalidSignature | AppError::UnknownSigner | AppError::Replay => {
                StatusCode::UNAUTHORIZED
            }
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if matches!(self, AppError::Internal(_)) {
            tracing::error!(error = %self, "internal error");
        }
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
