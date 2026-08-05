//! Not a CI test (ignored by default) -- a manual diagnostic against the
//! real deployed directory server, run on demand with:
//!   cargo test -p p2p-core --test real_server_smoke -- --ignored --nocapture
//! Exercises the exact same conditions a real user hits: two real macOS
//! processes, real sockets, the real remote directory server (not a
//! throwaway local one), fresh temp accounts so it never touches anyone's
//! real identity or interferes with an already-running instance.
use std::time::Duration;

use p2p_core::AppService;

const REAL_SERVER: &str = "https://seal.emn4tor.de";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn dm_round_trip_against_the_real_deployed_server() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let mut alice = AppService::load_or_create(
        alice_dir.path().to_path_buf(),
        REAL_SERVER.to_string(),
        Some("smoke-alice".to_string()),
    )
    .await
    .expect("alice registers with the real server");
    let mut bob = AppService::load_or_create(
        bob_dir.path().to_path_buf(),
        REAL_SERVER.to_string(),
        Some("smoke-bob".to_string()),
    )
    .await
    .expect("bob registers with the real server");

    let alice_id = alice.user_id();
    let bob_id = bob.user_id();
    eprintln!("alice={alice_id} bob={bob_id}");

    alice
        .add_contact_by_user_id(&bob_id)
        .await
        .expect("alice adds bob");
    alice
        .send_direct_message(&bob_id, "hello over the real server", None)
        .await
        .expect("alice sends");

    let result = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            tokio::select! {
                event = alice.next_event() => eprintln!("alice event: {event:?}"),
                event = bob.next_event() => {
                    eprintln!("bob event: {event:?}");
                    if let p2p_core::ChatEvent::DirectMessage { body, .. } = &event {
                        assert_eq!(body, "hello over the real server");
                        break;
                    }
                }
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "timed out waiting for bob to receive the DM"
    );
}
