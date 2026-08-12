use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::{AppService, ChatEvent};
use serial_test::serial;
use tokio::sync::{mpsc, oneshot};

/// Spawns a real directory-server on loopback, returns its base URL.
/// Duplicated from `app_service.rs`: no test-support crate to share it in.
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

/// Seeds a fresh data directory with an existing identity pickle before
/// `load_or_create` touches it, standing in for QR pairing: this test cares
/// about what happens once two devices share an identity, not how.
async fn seed_identity(
    data_dir: &std::path::Path,
    user_id: &str,
    display_name: &str,
    pickle_json: &str,
) {
    std::fs::create_dir_all(data_dir).unwrap();
    let keychain = identity::Keychain::for_app_data_dir(data_dir).unwrap();
    let kek = tokio::task::spawn_blocking(move || keychain.load_or_create_kek())
        .await
        .unwrap()
        .unwrap();
    let store = storage::LocalStore::open(&data_dir.join("local.sqlite3"), kek).unwrap();
    store
        .save_identity(user_id, display_name, pickle_json, 0)
        .unwrap();
}

/// A single `select!` loop can't drive this: two `AppService`s can have an
/// event ready simultaneously, and `select!` would silently drop one. Each
/// node gets its own task draining `next_event()` into an unbounded queue.
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

/// Two of my own devices, same identity, both independently online and
/// reachable without clobbering each other's presence/OTK pool.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn a_contact_messaging_a_two_device_account_reaches_both_devices() {
    let directory_url = spawn_directory_server().await;

    let alice_desktop_dir = tempfile::tempdir().unwrap();
    let alice_phone_dir = tempfile::tempdir().unwrap();
    let carol_dir = tempfile::tempdir().unwrap();

    let alice_desktop_svc = AppService::load_or_create(
        alice_desktop_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice's desktop starts up");
    let alice_id = alice_desktop_svc.user_id();
    let alice_pickle = alice_desktop_svc
        .node
        .identity
        .pickle_to_json()
        .expect("pickle alice's identity");

    // "Pair" alice's phone: same identity, fresh local store/device_id.
    seed_identity(alice_phone_dir.path(), &alice_id, "alice", &alice_pickle).await;
    let alice_phone_svc = AppService::load_or_create(
        alice_phone_dir.path().to_path_buf(),
        directory_url.clone(),
        None,
    )
    .await
    .expect("alice's phone resumes the shared identity");
    assert_eq!(alice_phone_svc.user_id(), alice_id);
    assert_ne!(
        alice_phone_svc.node.device_id(),
        alice_desktop_svc.node.device_id(),
        "two devices of the same account must have distinct device ids"
    );

    let carol_svc = AppService::load_or_create(
        carol_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("carol".to_string()),
    )
    .await
    .expect("carol starts up");

    let carol = spawn_node(carol_svc);
    let mut desktop = spawn_node(alice_desktop_svc);
    let mut phone = spawn_node(alice_phone_svc);

    carol
        .add_contact(&alice_id)
        .await
        .expect("carol adds alice");
    carol
        .send_dm(&alice_id, "hello alice")
        .await
        .expect("carol sends to alice's account");

    let mut desktop_got_it = false;
    let mut phone_got_it = false;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        while !(desktop_got_it && phone_got_it) {
            tokio::select! {
                Some(event) = desktop.event_rx.recv() => {
                    if let ChatEvent::DirectMessage { body, .. } = event {
                        assert_eq!(body, "hello alice");
                        desktop_got_it = true;
                    }
                }
                Some(event) = phone.event_rx.recv() => {
                    if let ChatEvent::DirectMessage { body, .. } = event {
                        assert_eq!(body, "hello alice");
                        phone_got_it = true;
                    }
                }
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out: desktop_got_it={desktop_got_it}, phone_got_it={phone_got_it}"
    );
}
