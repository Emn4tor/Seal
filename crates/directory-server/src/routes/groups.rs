use axum::Json;
use axum::extract::{Path, State};
use wire_proto::{
    ChannelRecord, CreateChannelRequest, CreateGroupRequest, GroupRecord, GroupRole,
    RosterUpdateRequest,
};

use crate::error::AppError;
use crate::state::{AppState, now_secs};
use crate::{auth, db};

pub async fn create_group(
    State(state): State<AppState>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<Json<GroupRecord>, AppError> {
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;

    state
        .with_conn(move |conn| {
            let creator = db::get_user(conn, &req.creator_id)?.ok_or(AppError::UnknownSigner)?;
            let pubkey = auth::decode_pubkey(&creator.ed25519_key)?;
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.creator_id, &req.nonce, now)?;
            db::create_group(conn, &req.group_id, &req.name, &req.creator_id, now)
        })
        .await
        .map(Json)
}

pub async fn get_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<GroupRecord>, AppError> {
    state
        .with_conn(move |conn| db::get_group(conn, &group_id)?.ok_or(AppError::NotFound))
        .await
        .map(Json)
}

/// Owner-only, same rule as roster changes: creating a channel is a
/// structural change to the group, and this project doesn't have a
/// separate "admin" tier — just Owner and Member.
pub async fn create_channel(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<Json<ChannelRecord>, AppError> {
    if group_id != req.group_id {
        return Err(AppError::BadRequest(
            "path group_id does not match request body".into(),
        ));
    }
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;

    state
        .with_conn(move |conn| {
            let actor = db::get_user(conn, &req.actor_id)?.ok_or(AppError::UnknownSigner)?;
            let pubkey = auth::decode_pubkey(&actor.ed25519_key)?;
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.actor_id, &req.nonce, now)?;

            let role = db::member_role(conn, &req.group_id, &req.actor_id)?
                .ok_or_else(|| AppError::Forbidden("actor is not a member of this group".into()))?;
            if role != GroupRole::Owner {
                return Err(AppError::Forbidden(
                    "only the group owner can create channels".into(),
                ));
            }

            db::create_channel(
                conn,
                &req.group_id,
                &req.channel_id,
                &req.name,
                req.kind,
                now,
            )
        })
        .await
        .map(Json)
}

pub async fn update_roster(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    Json(req): Json<RosterUpdateRequest>,
) -> Result<Json<GroupRecord>, AppError> {
    if group_id != req.group_id {
        return Err(AppError::BadRequest(
            "path group_id does not match request body".into(),
        ));
    }
    let now = now_secs();
    auth::check_timestamp(req.timestamp, now)?;

    state
        .with_conn(move |conn| {
            let actor = db::get_user(conn, &req.actor_id)?.ok_or(AppError::UnknownSigner)?;
            let pubkey = auth::decode_pubkey(&actor.ed25519_key)?;
            auth::verify_signature(&pubkey, &req.signing_bytes(), &req.signature)?;
            db::record_nonce_or_reject(conn, &req.actor_id, &req.nonce, now)?;

            let role = db::member_role(conn, &req.group_id, &req.actor_id)?
                .ok_or_else(|| AppError::Forbidden("actor is not a member of this group".into()))?;

            // Owners can add or remove anyone. Non-owner members may only
            // remove themselves (leaving the group) — they can't add others
            // or remove other members.
            if role != GroupRole::Owner {
                let only_self_removal =
                    req.add.is_empty() && req.remove.len() == 1 && req.remove[0] == req.actor_id;
                if !only_self_removal {
                    return Err(AppError::Forbidden(
                        "only the group owner can add or remove other members".into(),
                    ));
                }
            }

            let current_version =
                db::roster_version(conn, &req.group_id)?.ok_or(AppError::NotFound)?;
            if current_version != req.expected_version {
                return Err(AppError::Conflict(format!(
                    "roster has moved on: expected version {}, current version {}",
                    req.expected_version, current_version
                )));
            }

            db::apply_roster_update(
                conn,
                &req.group_id,
                &req.add,
                &req.remove,
                current_version + 1,
                now,
            )
        })
        .await
        .map(Json)
}
