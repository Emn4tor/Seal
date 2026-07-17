use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::post;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Clone)]
pub struct AdminState {
    pub app: AppState,
    pub token: String,
}

pub fn build_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/purge", post(purge_handler))
        .with_state(state)
}

async fn purge_handler(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let authorized = match provided {
        Some(token) => constant_time_eq(token.as_bytes(), state.token.as_bytes()),
        None => false,
    };
    if !authorized {
        return Err(AppError::Forbidden(
            "missing or invalid admin bearer token".into(),
        ));
    }

    state.app.purge().await?;
    tracing::warn!("directory database purged via admin endpoint");
    Ok(StatusCode::OK)
}

/// Avoids leaking token length/prefix information through comparison timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
