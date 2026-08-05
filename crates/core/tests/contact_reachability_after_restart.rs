use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::AppService;
use tokio::sync::{mpsc, oneshot};

/// Regression test for a real bug: `AppService::ensure_connected_contact`
/// used to only resolve a contact's dialable address the *first* time it
/// was ever sent to, then trust that cached address for the rest of the
/// process's lifetime. A peer's address is ephemeral (a fresh listen port
/// is bound on every launch), so as soon as the other side restarted —
/// something that happens constantly in `npm run tauri dev`, and even more
/// so once a person hits the "can't run two dev instances" port conflict
/// and starts killing/restarting processes to work around it — every send
/// after that silently failed to dial, with no recovery short of removing
/// and re-adding the contact. See `AppService::ensure_connected_contact`'s
/// doc comment for the fix and why it's gated on connection state rather
/// than refreshing unconditionally on every send.
///
/// Structured like `actor_parity_harness.rs`: each side gets its own
/// continuously-polling background task (command handling interleaved with
/// `next_event()` via an mpsc channel), matching
/// `apps/desktop/src-tauri/src/actor.rs`'s real structure — libp2p's Swarm
/// only makes progress while something is actively polling it.
enum Cmd {
    SendDm(String, String, oneshot::Sender<anyhow::Result<()>>),
    ListMessages(
        String,
        oneshot::Sender<anyhow::Result<Vec<storage::StoredMessage>>>,
    ),
}

struct Handle {
    tx: mpsc::Sender<Cmd>,
}

impl Handle {
    async fn send_dm(&self, peer: &str, body: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::SendDm(peer.to_string(), body.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    async fn list_messages(&self, conv: &str) -> anyhow::Result<Vec<storage::StoredMessage>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::ListMessages(conv.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    /// Stops this side's actor loop by dropping the sender half, which
    /// makes the background task's `rx.recv()` return `None` and return.
    fn shutdown(self) {
        drop(self.tx);
    }
}

fn spawn_actor(mut svc: AppService) -> Handle {
    let (tx, mut rx) = mpsc::channel::<Cmd>(32);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                maybe_cmd = rx.recv() => {
                    let Some(cmd) = maybe_cmd else { return };
                    match cmd {
                        Cmd::SendDm(peer, body, respond) => {
                            let _ = respond.send(svc.send_direct_message(&peer, &body, None).await);
                        }
                        Cmd::ListMessages(conv, respond) => {
                            let _ = respond.send(svc.list_messages(&conv));
                        }
                    }
                }
                // Just needs draining to keep the swarm making progress,
                // same as `actor.rs::run`.
                _ = svc.next_event() => {}
            }
        }
    });
    Handle { tx }
}

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

/// Polls `handle.list_messages(conv)` until it has `want` entries, or
/// `timeout` elapses.
async fn wait_for_message_count(
    handle: &Handle,
    conv: &str,
    want: usize,
    timeout: Duration,
) -> Vec<storage::StoredMessage> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let msgs = handle.list_messages(conv).await.unwrap();
        if msgs.len() >= want || tokio::time::Instant::now() >= deadline {
            return msgs;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// Ignored on Linux CI: creates real `AppService`s, which need a working OS
// keychain — see `actor_parity_harness.rs`'s identical comment for why.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn messaging_recovers_after_the_other_side_restarts() {
    let directory_url = spawn_directory_server().await;

    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();

    let alice_svc = AppService::load_or_create(
        alice_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice starts up");
    let bob_svc = AppService::load_or_create(
        bob_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("bob".to_string()),
    )
    .await
    .expect("bob starts up");

    let alice_id = alice_svc.user_id();
    let bob_id = bob_svc.user_id();

    let alice = spawn_actor(alice_svc);
    let mut bob = spawn_actor(bob_svc);

    tokio::time::sleep(Duration::from_millis(200)).await;

    alice
        .send_dm(&bob_id, "hello bob")
        .await
        .expect("alice sends first message");
    let bob_history = wait_for_message_count(&bob, &alice_id, 1, Duration::from_secs(10)).await;
    assert_eq!(
        bob_history.len(),
        1,
        "bob should receive the first message fine"
    );

    // Restart bob: stop his actor loop and load a fresh AppService from the
    // SAME data dir, exactly like relaunching the app -- identity,
    // contacts, and message history all persist (they're on disk), but his
    // listen port (and so his dialable address) is fresh. Alice's process
    // never restarts, so from her side nothing looks any different -- she
    // still has bob cached as a contact from before.
    bob.shutdown();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let bob_svc_restarted =
        AppService::load_or_create(bob_dir.path().to_path_buf(), directory_url, None)
            .await
            .expect("bob resumes after restart");
    bob = spawn_actor(bob_svc_restarted);
    tokio::time::sleep(Duration::from_millis(200)).await;

    alice
        .send_dm(&bob_id, "still there?")
        .await
        .expect("alice sends a second message after bob's restart");

    let bob_history_2 = wait_for_message_count(&bob, &alice_id, 2, Duration::from_secs(10)).await;
    assert_eq!(
        bob_history_2.len(),
        2,
        "bob should receive alice's message even though his address changed on restart"
    );
}
