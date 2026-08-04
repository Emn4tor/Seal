use axum::Json;
use axum::extract::State;
use wire_proto::RelayInfoResponse;

use crate::error::AppError;
use crate::state::AppState;

/// Public, unauthenticated — this is the relay's own dial-in address, not
/// anything about a specific user. 404 means this server isn't running a
/// relay (or its operator hasn't configured an externally-reachable
/// address for it yet): clients should treat that as "no relay available
/// right now", the same as any other best-effort NAT-traversal fallback.
pub async fn get_relay_info(
    State(state): State<AppState>,
) -> Result<Json<RelayInfoResponse>, AppError> {
    state.relay_info.clone().map(Json).ok_or(AppError::NotFound)
}
