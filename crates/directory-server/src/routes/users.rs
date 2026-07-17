use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use wire_proto::{ClaimedOtk, RegisterUserRequest, UploadOtkRequest, UserRecord};

use crate::error::AppError;
use crate::state::{AppState, now_secs};
use crate::{auth, db};

pub async fn register_user(
    State(state): State<AppState>,
    Json(req): Json<RegisterUserRequest>,
) -> Result<Json<UserRecord>, AppError> {
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;

    let pubkey = auth::decode_pubkey(&req.ed25519_key)?;
    auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;

    let expected_id = wire_proto::user_id_from_ed25519(pubkey.as_bytes());
    if expected_id != req.user_id {
        return Err(AppError::BadRequest(
            "user_id does not match the ed25519 key fingerprint".into(),
        ));
    }

    state
        .with_conn(move |conn| {
            db::record_nonce_or_reject(conn, &req.user_id, &req.nonce, now)?;
            db::insert_user(
                conn,
                &req.user_id,
                &req.display_name,
                &req.ed25519_key,
                &req.curve25519_key,
                now,
            )?;
            Ok(UserRecord {
                user_id: req.user_id,
                display_name: req.display_name,
                ed25519_key: req.ed25519_key,
                curve25519_key: req.curve25519_key,
            })
        })
        .await
        .map(Json)
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<UserRecord>, AppError> {
    state
        .with_conn(move |conn| db::get_user(conn, &user_id)?.ok_or(AppError::NotFound))
        .await
        .map(Json)
}

pub async fn upload_otk(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<UploadOtkRequest>,
) -> Result<StatusCode, AppError> {
    if user_id != req.user_id {
        return Err(AppError::BadRequest(
            "path user_id does not match request body".into(),
        ));
    }
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;

    state
        .with_conn(move |conn| {
            let user = db::get_user(conn, &req.user_id)?.ok_or(AppError::UnknownSigner)?;
            let pubkey = auth::decode_pubkey(&user.ed25519_key)?;
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.user_id, &req.nonce, now)?;
            db::upload_otks(
                conn,
                &req.user_id,
                &req.keys,
                req.fallback_key.as_ref(),
                now,
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn claim_otk(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<ClaimedOtk>, AppError> {
    state
        .with_conn(move |conn| {
            db::get_user(conn, &user_id)?.ok_or(AppError::NotFound)?;
            db::claim_otk(conn, &user_id)?.ok_or(AppError::NotFound)
        })
        .await
        .map(Json)
}
