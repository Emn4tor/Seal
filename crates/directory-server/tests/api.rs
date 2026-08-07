use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use directory_server::admin::{AdminState, build_admin_router};
use directory_server::{AppState, build_public_router};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use serde_json::{Value, json};
use tower::ServiceExt;
use wire_proto::{
    ChannelKind, ChannelRecord, ClaimedOtk, CreateChannelRequest, CreateGroupRequest, GroupRecord,
    OneTimeKeyEntry, PresenceRecord, PresenceUpdateRequest, RegisterUserRequest,
    RosterUpdateRequest, UploadOtkRequest, UserRecord,
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn new_identity() -> (SigningKey, String) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let user_id = wire_proto::user_id_from_ed25519(signing_key.verifying_key().as_bytes());
    (signing_key, user_id)
}

fn register_request(
    signing_key: &SigningKey,
    user_id: &str,
    display_name: &str,
) -> RegisterUserRequest {
    let mut req = RegisterUserRequest {
        user_id: user_id.to_string(),
        display_name: display_name.to_string(),
        ed25519_key: b64(signing_key.verifying_key().as_bytes()),
        curve25519_key: b64(&[7u8; 32]),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    req.signature = b64(&signing_key.sign(&req.signing_bytes()).to_bytes());
    req
}

async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    let req = builder.body(body).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

async fn test_state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.sqlite3");
    let state = AppState::open(db_path).unwrap();
    (state, dir)
}

async fn register(router: &Router, signing_key: &SigningKey, user_id: &str, name: &str) {
    let req = register_request(signing_key, user_id, name);
    let (status, _) = call(router, "POST", "/v1/users", Some(json!(req))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn register_and_fetch_user() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();

    register(&router, &signing_key, &user_id, "alice").await;

    let (status, body) = call(&router, "GET", &format!("/v1/users/{user_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let record: UserRecord = serde_json::from_value(body).unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.display_name, "alice");
}

#[tokio::test]
async fn re_registering_same_identity_is_idempotent() {
    // Regression test: a client re-asserts registration on every app
    // launch (the directory may have been purged since last time), using
    // the same user_id every time since it's derived from the identity's
    // own key. That must keep succeeding against a directory that still
    // has the previous launch's row, not 409.
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();

    register(&router, &signing_key, &user_id, "alice").await;
    register(&router, &signing_key, &user_id, "alice-renamed").await;

    let (status, body) = call(&router, "GET", &format!("/v1/users/{user_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let record: UserRecord = serde_json::from_value(body).unwrap();
    assert_eq!(record.display_name, "alice-renamed");
}

#[tokio::test]
async fn rejects_bad_signature() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();

    let mut req = register_request(&signing_key, &user_id, "alice");
    req.signature = b64(&[0u8; 64]); // corrupt the signature

    let (status, _) = call(&router, "POST", "/v1/users", Some(json!(req))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_replayed_nonce() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();

    let req = register_request(&signing_key, &user_id, "alice");
    let (status, _) = call(&router, "POST", "/v1/users", Some(json!(req.clone()))).await;
    assert_eq!(status, StatusCode::OK);

    // Same request (including the same nonce) sent again must be rejected,
    // not silently accepted or treated as an idempotent no-op.
    let (status, _) = call(&router, "POST", "/v1/users", Some(json!(req))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_mismatched_user_id() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, _correct_id) = new_identity();

    // Claim someone else's fingerprint as the user_id.
    let req = register_request(&signing_key, "not-my-fingerprint", "eve");
    let (status, _) = call(&router, "POST", "/v1/users", Some(json!(req))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn otk_upload_then_claim_one_time_then_fallback() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();
    register(&router, &signing_key, &user_id, "alice").await;

    let mut upload = UploadOtkRequest {
        user_id: user_id.clone(),
        keys: vec![OneTimeKeyEntry {
            key_id: "otk-1".into(),
            public_key: b64(&[1u8; 32]),
        }],
        fallback_key: Some(OneTimeKeyEntry {
            key_id: "fallback-1".into(),
            public_key: b64(&[2u8; 32]),
        }),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    upload.signature = b64(&signing_key.sign(&upload.signing_bytes()).to_bytes());

    let (status, _) = call(
        &router,
        "POST",
        &format!("/v1/users/{user_id}/otk"),
        Some(json!(upload)),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // First claim consumes the single-use one-time key.
    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{user_id}/otk/claim"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claimed: ClaimedOtk = serde_json::from_value(body).unwrap();
    assert_eq!(claimed.key_id, "otk-1");
    assert!(!claimed.is_fallback);

    // Second claim has no one-time keys left, falls back to the reusable fallback key.
    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{user_id}/otk/claim"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claimed: ClaimedOtk = serde_json::from_value(body).unwrap();
    assert_eq!(claimed.key_id, "fallback-1");
    assert!(claimed.is_fallback);

    // Fallback key is reusable — claiming again yields the same key again.
    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{user_id}/otk/claim"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claimed: ClaimedOtk = serde_json::from_value(body).unwrap();
    assert_eq!(claimed.key_id, "fallback-1");
}

#[tokio::test]
async fn presence_put_and_get_respects_ttl_cap() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();
    register(&router, &signing_key, &user_id, "alice").await;

    let mut req = PresenceUpdateRequest {
        user_id: user_id.clone(),
        peer_id: "12D3KooWabc".into(),
        multiaddrs: vec!["/ip4/127.0.0.1/udp/4001/quic-v1".into()],
        relay_addrs: vec![],
        ttl_secs: 999_999, // far above the server's cap
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    req.signature = b64(&signing_key.sign(&req.signing_bytes()).to_bytes());

    let before = now();
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/v1/presence/{user_id}"),
        Some(json!(req)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let record: PresenceRecord = serde_json::from_value(body).unwrap();
    assert!(
        record.expires_at <= before + 300,
        "TTL must be capped at 300s server-side"
    );

    let (status, body) = call(&router, "GET", &format!("/v1/presence/{user_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let fetched: PresenceRecord = serde_json::from_value(body).unwrap();
    assert_eq!(fetched.peer_id, "12D3KooWabc");
}

/// One identity flooding signed writes gets cut off rather than accepted
/// forever. See `db::record_nonce_or_reject`'s piggyback on the nonce
/// table for how this is enforced with no new schema or middleware.
#[tokio::test]
async fn one_identity_flooding_presence_updates_gets_rate_limited() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);
    let (signing_key, user_id) = new_identity();
    register(&router, &signing_key, &user_id, "alice").await;

    let mut last_status = StatusCode::OK;
    for _ in 0..(directory_server::db::MAX_REQUESTS_PER_WINDOW + 5) {
        let mut req = PresenceUpdateRequest {
            user_id: user_id.clone(),
            peer_id: "12D3KooWabc".into(),
            multiaddrs: vec![],
            relay_addrs: vec![],
            ttl_secs: 60,
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = b64(&signing_key.sign(&req.signing_bytes()).to_bytes());
        let (status, _) = call(
            &router,
            "PUT",
            &format!("/v1/presence/{user_id}"),
            Some(json!(req)),
        )
        .await;
        last_status = status;
    }
    assert_eq!(
        last_status,
        StatusCode::TOO_MANY_REQUESTS,
        "a flood of signed requests from one identity must eventually be rejected"
    );
}

#[tokio::test]
async fn group_lifecycle_and_roster_permissions() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);

    let (owner_key, owner_id) = new_identity();
    let (member_key, member_id) = new_identity();
    let (outsider_key, outsider_id) = new_identity();
    register(&router, &owner_key, &owner_id, "owner").await;
    register(&router, &member_key, &member_id, "member").await;
    register(&router, &outsider_key, &outsider_id, "outsider").await;

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut create = CreateGroupRequest {
        group_id: group_id.clone(),
        name: "test group".into(),
        creator_id: owner_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    create.signature = b64(&owner_key.sign(&create.signing_bytes()).to_bytes());
    let (status, body) = call(&router, "POST", "/v1/groups", Some(json!(create))).await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();
    assert_eq!(group.roster_version, 1);
    assert_eq!(group.members.len(), 1);

    // A non-owner cannot add other members.
    let mut bad_add = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: outsider_id.clone(),
        add: vec![outsider_id.clone()],
        remove: vec![],
        expected_version: 1,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    bad_add.signature = b64(&outsider_key.sign(&bad_add.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(bad_add)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner adds member_id.
    let mut add = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: owner_id.clone(),
        add: vec![member_id.clone()],
        remove: vec![],
        expected_version: 1,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    add.signature = b64(&owner_key.sign(&add.signing_bytes()).to_bytes());
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(add)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();
    assert_eq!(group.roster_version, 2);
    assert_eq!(group.members.len(), 2);

    // Stale expected_version is rejected as a conflict.
    let mut stale = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: owner_id.clone(),
        add: vec![],
        remove: vec![member_id.clone()],
        expected_version: 1, // stale: current version is now 2
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    stale.signature = b64(&owner_key.sign(&stale.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(stale)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Member can remove themselves (leave).
    let mut leave = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: member_id.clone(),
        add: vec![],
        remove: vec![member_id.clone()],
        expected_version: 2,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    leave.signature = b64(&member_key.sign(&leave.signing_bytes()).to_bytes());
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(leave)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();
    assert_eq!(group.roster_version, 3);
    assert_eq!(group.members.len(), 1);
    assert_eq!(group.members[0].user_id, owner_id);
}

#[tokio::test]
async fn adding_an_unregistered_user_id_to_a_roster_is_rejected() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);

    let (owner_key, owner_id) = new_identity();
    register(&router, &owner_key, &owner_id, "owner").await;

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut create = CreateGroupRequest {
        group_id: group_id.clone(),
        name: "test group".into(),
        creator_id: owner_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    create.signature = b64(&owner_key.sign(&create.signing_bytes()).to_bytes());
    let (status, _) = call(&router, "POST", "/v1/groups", Some(json!(create))).await;
    assert_eq!(status, StatusCode::OK);

    // "nobody" was never registered via `/v1/users`.
    let (_, phantom_id) = new_identity();
    let mut add = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: owner_id.clone(),
        add: vec![phantom_id],
        remove: vec![],
        expected_version: 1,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    add.signature = b64(&owner_key.sign(&add.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(add)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a roster can't gain a member who was never actually registered"
    );

    let (status, body) = call(&router, "GET", &format!("/v1/groups/{group_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();
    assert_eq!(
        group.members.len(),
        1,
        "the rejected add must not have partially applied"
    );
}

/// Exercises `/v1/users/{id}/groups` — the endpoint a client uses to
/// discover memberships it doesn't have locally (e.g. it missed the P2P
/// key-share that normally delivers them). Membership should show up for
/// everyone actually on the roster, nobody else, and drop off again once
/// someone leaves.
#[tokio::test]
async fn list_my_groups_reflects_membership() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);

    let (owner_key, owner_id) = new_identity();
    let (member_key, member_id) = new_identity();
    let (outsider_key, outsider_id) = new_identity();
    register(&router, &owner_key, &owner_id, "owner").await;
    register(&router, &member_key, &member_id, "member").await;
    register(&router, &outsider_key, &outsider_id, "outsider").await;

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut create = CreateGroupRequest {
        group_id: group_id.clone(),
        name: "test group".into(),
        creator_id: owner_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    create.signature = b64(&owner_key.sign(&create.signing_bytes()).to_bytes());
    let (status, _) = call(&router, "POST", "/v1/groups", Some(json!(create))).await;
    assert_eq!(status, StatusCode::OK);

    // Before being added, member_id sees no groups.
    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{member_id}/groups"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    let mut add = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: owner_id.clone(),
        add: vec![member_id.clone()],
        remove: vec![],
        expected_version: 1,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    add.signature = b64(&owner_key.sign(&add.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(add)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Owner and member both see it now; the outsider still doesn't.
    for user_id in [&owner_id, &member_id] {
        let (status, body) =
            call(&router, "GET", &format!("/v1/users/{user_id}/groups"), None).await;
        assert_eq!(status, StatusCode::OK);
        let group_ids: Vec<String> = serde_json::from_value(body).unwrap();
        assert_eq!(group_ids, vec![group_id.clone()]);
    }
    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{outsider_id}/groups"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    // member_id leaves; it drops back out of their list.
    let mut leave = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: member_id.clone(),
        add: vec![],
        remove: vec![member_id.clone()],
        expected_version: 2,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    leave.signature = b64(&member_key.sign(&leave.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(leave)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(
        &router,
        "GET",
        &format!("/v1/users/{member_id}/groups"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn group_creation_seeds_a_default_channel_and_channel_creation_is_owner_only() {
    let (state, _dir) = test_state().await;
    let router = build_public_router(state);

    let (owner_key, owner_id) = new_identity();
    let (member_key, member_id) = new_identity();
    register(&router, &owner_key, &owner_id, "owner").await;
    register(&router, &member_key, &member_id, "member").await;

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut create = CreateGroupRequest {
        group_id: group_id.clone(),
        name: "test group".into(),
        creator_id: owner_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    create.signature = b64(&owner_key.sign(&create.signing_bytes()).to_bytes());
    let (status, body) = call(&router, "POST", "/v1/groups", Some(json!(create))).await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();

    // A group is never channel-less — it starts with a default "general"
    // text channel.
    assert_eq!(group.channels.len(), 1);
    assert_eq!(group.channels[0].name, "general");
    assert_eq!(group.channels[0].kind, ChannelKind::Text);

    // Owner adds member_id so there's a non-owner to test against.
    let mut add = RosterUpdateRequest {
        group_id: group_id.clone(),
        actor_id: owner_id.clone(),
        add: vec![member_id.clone()],
        remove: vec![],
        expected_version: 1,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    add.signature = b64(&owner_key.sign(&add.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/v1/groups/{group_id}"),
        Some(json!(add)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A non-owner member cannot create a channel.
    let mut bad_create = CreateChannelRequest {
        group_id: group_id.clone(),
        channel_id: uuid::Uuid::new_v4().to_string(),
        name: "sneaky".into(),
        kind: ChannelKind::Text,
        actor_id: member_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    bad_create.signature = b64(&member_key.sign(&bad_create.signing_bytes()).to_bytes());
    let (status, _) = call(
        &router,
        "POST",
        &format!("/v1/groups/{group_id}/channels"),
        Some(json!(bad_create)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner can create a voice channel.
    let mut create_voice = CreateChannelRequest {
        group_id: group_id.clone(),
        channel_id: uuid::Uuid::new_v4().to_string(),
        name: "hangout".into(),
        kind: ChannelKind::Voice,
        actor_id: owner_id.clone(),
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    create_voice.signature = b64(&owner_key.sign(&create_voice.signing_bytes()).to_bytes());
    let (status, body) = call(
        &router,
        "POST",
        &format!("/v1/groups/{group_id}/channels"),
        Some(json!(create_voice)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let channel: ChannelRecord = serde_json::from_value(body).unwrap();
    assert_eq!(channel.name, "hangout");
    assert_eq!(channel.kind, ChannelKind::Voice);
    assert_eq!(channel.position, 1); // after the default "general" at 0

    // The new channel shows up when fetching the group.
    let (status, body) = call(&router, "GET", &format!("/v1/groups/{group_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let group: GroupRecord = serde_json::from_value(body).unwrap();
    assert_eq!(group.channels.len(), 2);
    assert!(group.channels.iter().any(|c| c.name == "hangout"));
}

#[tokio::test]
async fn admin_purge_wipes_everything_instantly() {
    let (state, _dir) = test_state().await;
    let public_router = build_public_router(state.clone());
    let admin_router = build_admin_router(AdminState {
        app: state,
        token: "test-token".into(),
    });

    let (signing_key, user_id) = new_identity();
    register(&public_router, &signing_key, &user_id, "alice").await;
    let (status, _) = call(&public_router, "GET", &format!("/v1/users/{user_id}"), None).await;
    assert_eq!(status, StatusCode::OK);

    // Wrong token is rejected.
    let req = Request::builder()
        .method("POST")
        .uri("/admin/purge")
        .header("authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();
    let resp = admin_router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Correct token purges everything.
    let req = Request::builder()
        .method("POST")
        .uri("/admin/purge")
        .header("authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();
    let resp = admin_router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let (status, _) = call(&public_router, "GET", &format!("/v1/users/{user_id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
