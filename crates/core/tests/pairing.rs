use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::{AppService, ChatEvent};
use serial_test::serial;
use tokio::sync::{mpsc, oneshot};

/// Spawns a real directory-server on loopback and returns its base URL. Same
/// helper as `app_service.rs`'s/`multi_device.rs`'s own.
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

/// Same reasoning as `multi_device.rs`'s identical harness: each node
/// drains `next_event()` on its own task so nothing is lost to `select!`.
enum Cmd {
    AddContact(String, oneshot::Sender<anyhow::Result<()>>),
    SendDm(String, String, oneshot::Sender<anyhow::Result<()>>),
}

struct NodeHandle {
    cmd_tx: mpsc::Sender<Cmd>,
    event_rx: mpsc::UnboundedReceiver<ChatEvent>,
}

impl NodeHandle {
    async fn add_contact(&self, id: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::AddContact(id.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn send_dm(&self, peer: &str, body: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::SendDm(peer.to_string(), body.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
}

fn spawn_node(mut svc: AppService) -> NodeHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(8);
    let (event_tx, event_rx) = mpsc::unbounded_channel::<ChatEvent>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                maybe_cmd = cmd_rx.recv() => {
                    let Some(cmd) = maybe_cmd else { return };
                    match cmd {
                        Cmd::AddContact(id, respond) => {
                            let _ = respond.send(svc.add_contact_by_user_id(&id).await);
                        }
                        Cmd::SendDm(peer, body, respond) => {
                            let _ = respond.send(svc.send_direct_message(&peer, &body, None).await);
                        }
                    }
                }
                event = svc.next_event() => {
                    let _ = event_tx.send(event);
                }
            }
        }
    });
    NodeHandle { cmd_tx, event_rx }
}

/// Drives a bare event-draining loop with no command channel — used for the
/// inviting device here, which only needs to keep answering pairing/network
/// events, never anything the test issues commands to it for directly.
fn spawn_draining(mut svc: AppService) {
    tokio::spawn(async move {
        loop {
            let _ = svc.next_event().await;
        }
    });
}

/// Full QR-pairing ceremony: alice's phone joins via `join_via_pairing`
/// and must end up an independent second device of the same account,
/// able to message a bootstrapped contact and be reachable by it.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn qr_pairing_produces_a_working_second_device() {
    let directory_url = spawn_directory_server().await;

    let alice_desktop_dir = tempfile::tempdir().unwrap();
    let alice_phone_dir = tempfile::tempdir().unwrap();
    let carol_dir = tempfile::tempdir().unwrap();

    let mut alice_desktop_svc = AppService::load_or_create(
        alice_desktop_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice's desktop starts up");
    let alice_id = alice_desktop_svc.user_id();
    let alice_desktop_device_id = alice_desktop_svc.node.device_id().to_string();

    let carol_svc = AppService::load_or_create(
        carol_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("carol".to_string()),
    )
    .await
    .expect("carol starts up");
    let carol_id = carol_svc.user_id();

    // Alice knows carol *before* pairing, proving the bootstrap actually
    // carries contacts across rather than the phone self-healing later.
    alice_desktop_svc
        .add_contact_by_user_id(&carol_id)
        .await
        .expect("alice adds carol before pairing");

    let offer = alice_desktop_svc
        .start_pairing()
        .expect("alice starts a pairing offer");

    // From here on, alice's desktop just needs to keep answering events
    // (including the pairing request itself) — nothing in this test issues
    // it further commands.
    spawn_draining(alice_desktop_svc);

    let phone_svc = AppService::join_via_pairing(
        alice_phone_dir.path().to_path_buf(),
        directory_url.clone(),
        &offer,
    )
    .await
    .expect("alice's phone joins via pairing");

    assert_eq!(
        phone_svc.user_id(),
        alice_id,
        "the paired device must share the same account"
    );
    assert_ne!(
        phone_svc.node.device_id(),
        alice_desktop_device_id,
        "the paired device must be a distinct device, not a clone of the inviting one"
    );

    let mut phone = spawn_node(phone_svc);
    let mut carol = spawn_node(carol_svc);

    // The phone can message carol *without* ever calling add_contact
    // itself — proving the bootstrap snapshot from pairing actually seeded
    // `contacts`/`contact_devices` locally.
    phone
        .send_dm(&carol_id, "hello from alice's phone")
        .await
        .expect("phone sends to a contact it only knows via pairing bootstrap");

    let mut carol_got_it = false;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        while !carol_got_it {
            if let Some(event) = carol.event_rx.recv().await
                && let ChatEvent::DirectMessage { from, body, .. } = event
            {
                assert_eq!(from, alice_id);
                assert_eq!(body, "hello from alice's phone");
                carol_got_it = true;
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for carol to receive the phone's message"
    );

    // And carol can reach the phone directly too — proving the inviting
    // device actually registered the phone's certificate with the
    // directory during the handshake, not just handed it a copy locally.
    carol
        .add_contact(&alice_id)
        .await
        .expect("carol (re-)resolves alice's account, now with two devices");
    carol
        .send_dm(&alice_id, "hi phone")
        .await
        .expect("carol sends to alice's account");

    let mut phone_got_it = false;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        while !phone_got_it {
            if let Some(event) = phone.event_rx.recv().await
                && let ChatEvent::DirectMessage { from, body, .. } = event
            {
                assert_eq!(from, carol_id);
                assert_eq!(body, "hi phone");
                phone_got_it = true;
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for the phone to receive carol's message — the directory likely never learned about the paired device's certificate"
    );
}
