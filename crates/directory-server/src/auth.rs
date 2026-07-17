use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::AppError;

/// Requests with a `timestamp` further than this from the server's clock are
/// rejected — bounds how long a captured (but validly signed) request could
/// be replayed even before nonce tracking is considered.
pub const MAX_CLOCK_SKEW_SECS: i64 = 300;

pub fn decode_pubkey(b64: &str) -> Result<VerifyingKey, AppError> {
    let bytes = STANDARD
        .decode(b64)
        .map_err(|_| AppError::BadRequest("invalid base64 key".into()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| AppError::BadRequest("ed25519 key must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|_| AppError::BadRequest("invalid ed25519 key".into()))
}

pub fn verify_signature(
    pubkey: &VerifyingKey,
    signing_bytes: &[u8],
    signature_b64: &str,
) -> Result<(), AppError> {
    let sig_bytes = STANDARD
        .decode(signature_b64)
        .map_err(|_| AppError::InvalidSignature)?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| AppError::InvalidSignature)?;
    let sig = Signature::from_bytes(&sig_arr);
    // `verify_strict` (vs. `verify`) rejects malleable/non-canonical signature
    // encodings — important here since signature verification *is* the auth
    // system (no separate password/account layer backs it up).
    pubkey
        .verify_strict(signing_bytes, &sig)
        .map_err(|_| AppError::InvalidSignature)
}

pub fn check_timestamp(ts: i64, now: i64) -> Result<(), AppError> {
    if (now - ts).abs() > MAX_CLOCK_SKEW_SECS {
        return Err(AppError::Replay);
    }
    Ok(())
}
