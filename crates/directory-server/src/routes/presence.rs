use axum::Json;
use axum::extract::{Path, State};
use wire_proto::{PresenceRecord, PresenceUpdateRequest};

use crate::error::AppError;
use crate::state::{AppState, now_secs};
use crate::{auth, db};

/// Presence TTLs are capped server-side regardless of what a client requests —
/// this is a rendezvous cache, not a durable registry, and a short cap keeps
/// stale addresses from lingering after a peer goes offline.
const MAX_PRESENCE_TTL_SECS: u64 = 300;

pub async fn put_presence(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<PresenceUpdateRequest>,
) -> Result<Json<PresenceRecord>, AppError> {
    if user_id != req.user_id {
        return Err(AppError::BadRequest(
            "path user_id does not match request body".into(),
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
                &req.peer_id,
                &req.multiaddrs,
                &req.relay_addrs,
                expires_at,
                now,
            )?;
            Ok(PresenceRecord {
                user_id: req.user_id,
                peer_id: req.peer_id,
                multiaddrs: req.multiaddrs,
                relay_addrs: req.relay_addrs,
                expires_at,
            })
        })
        .await
        .map(Json)
}

pub async fn get_presence(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<PresenceRecord>, AppError> {
    let now = now_secs();
    state
        .with_conn(move |conn| db::get_presence(conn, &user_id, now)?.ok_or(AppError::NotFound))
        .await
        .map(Json)
}
