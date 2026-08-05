use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::{AppService, ChatEvent};

/// Spawns a real directory-server on loopback and returns its base URL. Same
/// helper as `app_service.rs`/`voice_presence.rs` — the backing temp dir is
/// intentionally leaked for the test process's lifetime.
async fn spawn_directory_server() -> String {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("directory.sqlite3");
    let state = AppState::open(db_path).unwrap();
    let router = build_public_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    std::mem::forget(dir);
    format!("http://{addr}")
}

/// The 1:1-call counterpart to `voice_presence.rs`'s group-voice test: two
/// real `AppService`s, alice rings bob (nobody's messaged the other before —
/// exercises the same unknown-caller self-heal `DirectMessage` gets), bob
/// accepts, and both sides must end up with an active `Direct`-scoped voice
/// call to each other, discovered purely through the `CallInvite`/
/// `CallAccept` signaling (no group, no gossipsub topic involved at all).
// Ignored for the same reason as `voice.rs`/`voice_presence.rs`: starting
// the actual call opens the real default mic/speaker as a side effect of
// exercising the signaling logic this test cares about (audio I/O degrades
// gracefully with no device/permission). Run explicitly with
// `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "opens the real default mic/speaker; run explicitly with `cargo test -- --ignored`"]
async fn ringing_then_accepting_connects_a_direct_call() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let directory_url = spawn_directory_server().await;

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let mut alice = AppService::load_or_create(
        alice_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice starts up");
    let mut bob = AppService::load_or_create(
        bob_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("bob".to_string()),
    )
    .await
    .expect("bob starts up");

    let alice_id = alice.user_id();
    let bob_id = bob.user_id();

    // Alice knows bob's id (e.g. shared out of band) but has never added
    // him or messaged him — mirrors "call someone straight from their ID"
    // as well as "call a contact you've DMed before" (the DM-then-call
    // case is a strict subset: `ensure_connected_contact` already handles
    // both identically).
    let outgoing_call_id = alice
        .call_contact(&bob_id, None, None)
        .await
        .expect("alice calls bob");

    let mut bob_invited_call_id: Option<String> = None;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                _ = alice.next_event() => {}
                event = bob.next_event() => {
                    if let ChatEvent::CallInvited { from, call_id } = event {
                        assert_eq!(from, alice_id);
                        bob_invited_call_id = Some(call_id);
                        break;
                    }
                }
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for bob to see the incoming call"
    );
    let call_id = bob_invited_call_id.expect("bob saw a CallInvited event");
    assert_eq!(call_id, outgoing_call_id);

    bob.accept_call(&call_id, None, None)
        .await
        .expect("bob accepts the call");

    let mut alice_saw_accept = false;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if alice_saw_accept {
                break;
            }
            tokio::select! {
                event = alice.next_event() => {
                    if let ChatEvent::CallAccepted { from, call_id: id } = event {
                        assert_eq!(from, bob_id);
                        assert_eq!(id, call_id);
                        alice_saw_accept = true;
                    }
                }
                _ = bob.next_event() => {}
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for alice to see the call accepted"
    );

    // Both sides should now have an active direct call to each other. The
    // actual dial (`AppService::connect_voice_peer`'s tie-break, spawned
    // via `VoiceCallState::spawn_ensure_connected`) only makes progress
    // while each side's swarm is being polled — same as every other test
    // in this crate that waits on real network activity — so this has to
    // keep calling `next_event()` on both, not just sleep and re-check.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut both_connected = false;
    while !both_connected && tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = alice.next_event() => {}
            _ = bob.next_event() => {}
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
        both_connected = alice.voice_participants() == vec![bob_id.clone()]
            && bob.voice_participants() == vec![alice_id.clone()];
    }
    assert!(
        both_connected,
        "both sides should show the other as an active call participant"
    );

    alice.leave_voice_channel().expect("alice hangs up");
    bob.leave_voice_channel().expect("bob hangs up");
}

/// A declined call never starts audio on either side, and the caller learns
/// about the decline. No mic/speaker involved (the call never connects), so
/// this one isn't gated behind `--ignored` in general — only on Linux CI
/// specifically, since it still creates real `AppService`s that need a
/// working OS keychain (see `actor_parity_harness.rs`'s identical comment).
#[tokio::test]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn declining_a_call_never_connects_it() {
    let directory_url = spawn_directory_server().await;

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let mut alice = AppService::load_or_create(
        alice_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice starts up");
    let mut bob = AppService::load_or_create(
        bob_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("bob".to_string()),
    )
    .await
    .expect("bob starts up");

    let bob_id = bob.user_id();

    alice
        .call_contact(&bob_id, None, None)
        .await
        .expect("alice calls bob");

    let mut call_id: Option<String> = None;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                _ = alice.next_event() => {}
                event = bob.next_event() => {
                    if let ChatEvent::CallInvited { call_id: id, .. } = event {
                        call_id = Some(id);
                        break;
                    }
                }
            }
        }
    })
    .await;
    assert!(result.is_ok());
    let call_id = call_id.expect("bob saw the incoming call");

    bob.decline_call(&call_id).expect("bob declines");

    let mut alice_saw_decline = false;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if alice_saw_decline {
                break;
            }
            tokio::select! {
                event = alice.next_event() => {
                    if let ChatEvent::CallDeclined { from, call_id: id } = event {
                        assert_eq!(from, bob_id);
                        assert_eq!(id, call_id);
                        alice_saw_decline = true;
                    }
                }
                _ = bob.next_event() => {}
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for alice to see the decline"
    );

    assert!(alice.voice_participants().is_empty());
    assert!(bob.voice_participants().is_empty());
}
