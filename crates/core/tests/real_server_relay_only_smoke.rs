//! Not a CI test (ignored by default) -- manual diagnostic against the real
//! deployed directory server, run with:
//!   cargo test -p p2p-core --test real_server_relay_only_smoke -- --ignored --nocapture
//! Unlike `real_server_smoke.rs`, both sides use `simulate_wan` so they
//! withhold their real LAN/loopback address from presence entirely --
//! the only way they can reach each other is through the relay, exactly
//! like two people on genuinely different networks. This is what actually
//! proves the relay/dcutr path works end to end, not just that a
//! same-machine LAN/loopback dial papered over a broken relay.
use std::time::Duration;

use p2p_core::AppService;

const REAL_SERVER: &str = "https://seal.emn4tor.de";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn dm_round_trip_relay_only_against_the_real_deployed_server() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let mut alice = AppService::load_or_create_with(
        alice_dir.path().to_path_buf(),
        REAL_SERVER.to_string(),
        Some("relay-smoke-alice".to_string()),
        true,
    )
    .await
    .expect("alice registers with the real server, relay-only");
    let mut bob = AppService::load_or_create_with(
        bob_dir.path().to_path_buf(),
        REAL_SERVER.to_string(),
        Some("relay-smoke-bob".to_string()),
        true,
    )
    .await
    .expect("bob registers with the real server, relay-only");

    let alice_id = alice.user_id();
    let bob_id = bob.user_id();
    eprintln!("alice={alice_id} bob={bob_id}");

    alice
        .add_contact_by_user_id(&bob_id)
        .await
        .expect("alice adds bob");
    alice
        .send_direct_message(&bob_id, "hello purely through the relay", None)
        .await
        .expect("alice sends");

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                event = alice.next_event() => eprintln!("alice event: {event:?}"),
                event = bob.next_event() => {
                    eprintln!("bob event: {event:?}");
                    if let p2p_core::ChatEvent::DirectMessage { body, .. } = &event {
                        assert_eq!(body, "hello purely through the relay");
                        break;
                    }
                }
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "timed out waiting for bob to receive the relay-only DM"
    );
}
