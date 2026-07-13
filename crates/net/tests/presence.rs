use std::time::{SystemTime, UNIX_EPOCH};

use directory_server::{AppState, build_public_router};
use identity::Identity;
use net::presence::push_presence;
use wire_proto::{PresenceRecord, RegisterUserRequest};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn register_identity(base_url: &str, identity: &Identity) {
    let mut req = RegisterUserRequest {
        user_id: identity.user_id(),
        display_name: "tester".to_string(),
        ed25519_key: identity.ed25519_public_base64(),
        curve25519_key: identity.curve25519_public_base64(),
        timestamp: now(),
        nonce: "test-nonce-register".to_string(),
        signature: String::new(),
    };
    req.signature = identity.sign(&req.signing_bytes());

    let resp = reqwest::Client::new()
        .post(format!("{base_url}/v1/users"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "registration failed: {:?}",
        resp.text().await
    );
}

/// Exercises the real chain: `identity` crate signs a presence update,
/// `net::presence::push_presence` sends it over real HTTP, and an actual
/// running `directory-server` verifies and stores it — the same rendezvous
/// path the Tauri app will use once it's wired up in later phases.
#[tokio::test]
async fn presence_heartbeat_reaches_a_real_directory_server() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.sqlite3");
    let state = AppState::open(db_path).unwrap();
    let router = build_public_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base_url = format!("http://{addr}");

    let identity = Identity::generate();
    register_identity(&base_url, &identity).await;

    push_presence(
        &base_url,
        &identity,
        "12D3KooWTestPeerId",
        vec!["/ip4/127.0.0.1/udp/4001/quic-v1".to_string()],
        vec![],
        60,
    )
    .await
    .expect("presence push should succeed");

    let resp = reqwest::get(format!("{base_url}/v1/presence/{}", identity.user_id()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let record: PresenceRecord = resp.json().await.unwrap();
    assert_eq!(record.peer_id, "12D3KooWTestPeerId");
    assert_eq!(
        record.multiaddrs,
        vec!["/ip4/127.0.0.1/udp/4001/quic-v1".to_string()]
    );
}
