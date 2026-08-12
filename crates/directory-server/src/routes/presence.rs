use axum::Json;
use axum::extract::{Path, State};
use wire_proto::{PresenceListResponse, PresenceRecord, PresenceUpdateRequest};

use crate::error::AppError;
use crate::state::{AppState, now_secs};
use crate::{auth, db};

/// Presence TTLs are capped server-side regardless of what a client requests —
/// this is a rendezvous cache, not a durable registry, and a short cap keeps
/// stale addresses from lingering after a peer goes offline.
const MAX_PRESENCE_TTL_SECS: u64 = 300;

pub async fn put_presence(
    State(state): State<AppState>,
    Path((user_id, device_id)): Path<(String, String)>,
    Json(req): Json<PresenceUpdateRequest>,
) -> Result<Json<PresenceRecord>, AppError> {
    if user_id != req.user_id || device_id != req.device_id {
        return Err(AppError::BadRequest(
            "path does not match request body".into(),
        ));
    }
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;
    let ttl = req.ttl_secs.min(MAX_PRESENCE_TTL_SECS);

    state
        .with_conn(move |conn| {
            let user = db::get_user(conn, &req.user_id)?.ok_or(AppError::UnknownSigner)?;
            let pubkey = auth::decode_pubkey(&user.ed25519_key)?;
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.user_id, &req.nonce, now)?;
            let expires_at = now + ttl as i64;
            db::upsert_presence(
                conn,
                &req.user_id,
                &req.device_id,
                &req.peer_id,
                &req.multiaddrs,
                &req.relay_addrs,
                req.share_online_status,
                expires_at,
                now,
            )?;
            Ok(PresenceRecord {
                user_id: req.user_id,
                device_id: req.device_id,
                peer_id: req.peer_id,
                multiaddrs: req.multiaddrs,
                relay_addrs: req.relay_addrs,
                expires_at,
                share_online_status: req.share_online_status,
            })
        })
        .await
        .map(Json)
}

/// Every currently-live device of this account, for a sender to fan a
/// message out to. Replaces the old single-record `get_presence` now that
/// an account can have more than one reachable device at once.
pub async fn get_all_presence(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<PresenceListResponse>, AppError> {
    let now = now_secs();
    state
        .with_conn(move |conn| {
            let devices = db::get_presence_all(conn, &user_id, now)?;
            Ok(PresenceListResponse { devices })
        })
        .await
        .map(Json)
}
