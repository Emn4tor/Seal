//! Device registration verifies two signatures: the outer request's, and
//! the nested `DeviceCertificate`'s own. Without the second check, anyone
//! could graft a device onto someone else's account by naming their `user_id`.

use axum::Json;
use axum::extract::{Path, State};
use wire_proto::{DeviceCertificate, DeviceListResponse, RegisterDeviceRequest};

use crate::error::AppError;
use crate::state::{AppState, now_secs};
use crate::{auth, db};

pub async fn register_device(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(req): Json<RegisterDeviceRequest>,
) -> Result<Json<DeviceCertificate>, AppError> {
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
            // Outer request: the usual write-authorization check.
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.user_id, &req.nonce, now)?;

            // Nested cert must claim the master key already on file for this
            // account and be validly signed by that key — see module doc.
            if req.cert.master_ed25519_key != user.ed25519_key {
                return Err(AppError::BadRequest(
                    "device certificate's master key does not match this account".into(),
                ));
            }
            auth::verify_signature(&pubkey, &req.cert.signing_bytes(), &req.cert.signature)?;

            db::insert_device(
                conn,
                &req.user_id,
                &req.cert.device_id,
                &req.cert.device_ed25519_key,
                &req.cert.device_curve25519_key,
                &req.cert.master_ed25519_key,
                &req.cert.signature,
                now,
            )?;
            Ok(req.cert.clone())
        })
        .await
        .map(Json)
}

/// Public, unauthenticated read — same trust level as `get_user`. A
/// contact fetches this and verifies every cert themselves before trusting
/// it; nothing here is meant to be trusted just because the server served it.
pub async fn list_devices(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<DeviceListResponse>, AppError> {
    state
        .with_conn(move |conn| {
            let devices = db::list_devices(conn, &user_id)?;
            Ok(DeviceListResponse { devices })
        })
        .await
        .map(Json)
}
