use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::{AppService, ChatEvent};
use serial_test::serial;
use tokio::sync::{mpsc, oneshot};

/// Spawns a real directory-server on loopback, returns its base URL. Same
/// helper as the other integration tests here.
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
/// drains `next_event()` on its own task so `SyncCompleted` isn't lost.
enum Cmd {
    SendDm(String, String, oneshot::Sender<anyhow::Result<()>>),
    SyncWithDevice(String, oneshot::Sender<anyhow::Result<()>>),
    ListMessages(
        String,
        oneshot::Sender<anyhow::Result<Vec<storage::StoredMessage>>>,
    ),
    CreateGroup(String, oneshot::Sender<anyhow::Result<p2p_core::GroupInfo>>),
    SendGroupMessage(String, String, String, oneshot::Sender<anyhow::Result<()>>),
    ListGroups(oneshot::Sender<anyhow::Result<Vec<storage::StoredGroup>>>),
}

struct NodeHandle {
    cmd_tx: mpsc::Sender<Cmd>,
    event_rx: mpsc::UnboundedReceiver<ChatEvent>,
}

impl NodeHandle {
    async fn send_dm(&self, peer: &str, body: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::SendDm(peer.to_string(), body.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn sync_with_device(&self, peer_device_id: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::SyncWithDevice(peer_device_id.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn list_messages(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Vec<storage::StoredMessage>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::ListMessages(conversation_id.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn create_group(&self, name: &str) -> anyhow::Result<p2p_core::GroupInfo> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::CreateGroup(name.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn send_group_message(
        &self,
        group_id: &str,
        channel_id: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::SendGroupMessage(
                group_id.to_string(),
                channel_id.to_string(),
                body.to_string(),
                tx,
            ))
            .await
            .unwrap();
        rx.await.unwrap()
    }

    async fn list_groups(&self) -> anyhow::Result<Vec<storage::StoredGroup>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(Cmd::ListGroups(tx)).await.unwrap();
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
                        Cmd::SendDm(peer, body, respond) => {
                            let _ = respond.send(svc.send_direct_message(&peer, &body, None).await);
                        }
                        Cmd::SyncWithDevice(peer_device_id, respond) => {
                            let _ = respond.send(svc.sync_with_device(&peer_device_id).await);
                        }
                        Cmd::ListMessages(conversation_id, respond) => {
                            let _ = respond.send(svc.list_messages(&conversation_id));
                        }
                        Cmd::CreateGroup(name, respond) => {
                            let _ = respond.send(svc.create_group(&name).await);
                        }
                        Cmd::SendGroupMessage(group_id, channel_id, body, respond) => {
                            let _ = respond.send(
                                svc.send_group_message(&group_id, &channel_id, &body, None)
                                    .await,
                            );
                        }
                        Cmd::ListGroups(respond) => {
                            let _ = respond.send(svc.list_groups());
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

async fn wait_for_sync_completed(handle: &mut NodeHandle) {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(event) = handle.event_rx.recv().await
                && matches!(event, ChatEvent::SyncCompleted { .. })
            {
                return;
            }
        }
    })
    .await;
    assert!(result.is_ok(), "timed out waiting for sync to complete");
}

/// Manual sync between two paired devices converges to the union of what
/// each side had, in one round trip, and is idempotent: the "press Sync
/// on phone" flow end to end. `#[serial]`/`ignore`: see `pairing.rs`.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn manual_sync_converges_both_devices_to_the_union() {
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
    let alice_desktop_device_id = alice_desktop_svc.node.device_id().to_string();

    let carol_svc = AppService::load_or_create(
        carol_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("carol".to_string()),
    )
    .await
    .expect("carol starts up");
    let carol_id = carol_svc.user_id();

    alice_desktop_svc
        .add_contact_by_user_id(&carol_id)
        .await
        .expect("alice's desktop adds carol");

    // Sent before the phone exists at all, proving sync (not pairing
    // bootstrap, which never carries message history) brings it over.
    alice_desktop_svc
        .send_direct_message(&carol_id, "hello from desktop, before pairing", None)
        .await
        .expect("desktop sends carol a message before pairing");

    let offer = alice_desktop_svc
        .start_pairing()
        .expect("alice starts a pairing offer");

    let desktop = spawn_node(alice_desktop_svc);

    let phone_svc = AppService::join_via_pairing(
        alice_phone_dir.path().to_path_buf(),
        directory_url.clone(),
        &offer,
    )
    .await
    .expect("alice's phone joins via pairing");
    let phone_device_id = phone_svc.node.device_id().to_string();
    assert_ne!(phone_device_id, alice_desktop_device_id);

    let mut phone = spawn_node(phone_svc);

    // The phone doesn't know about the pre-pairing message yet — pairing's
    // bootstrap snapshot is contacts/groups only, never message history.
    let phone_messages_before_sync = phone
        .list_messages(&carol_id)
        .await
        .expect("phone lists its (empty) history with carol");
    assert!(
        phone_messages_before_sync.is_empty(),
        "the phone shouldn't have the desktop's pre-pairing message yet"
    );

    // The phone also has a message of its own the desktop has never seen —
    // this is what proves a single sync round trip converges *both*
    // directions at once, not just "pull from the other side."
    phone
        .send_dm(&carol_id, "hello from phone, after pairing")
        .await
        .expect("phone sends carol a message of its own");

    // Press "Sync" on the phone.
    phone
        .sync_with_device(&alice_desktop_device_id)
        .await
        .expect("phone requests a sync with the desktop");
    wait_for_sync_completed(&mut phone).await;

    let phone_messages = phone
        .list_messages(&carol_id)
        .await
        .expect("phone lists its history with carol after syncing");
    let phone_bodies: Vec<&str> = phone_messages.iter().map(|m| m.body.as_str()).collect();
    assert!(
        phone_bodies.contains(&"hello from desktop, before pairing"),
        "the phone should have picked up the desktop's pre-pairing message via sync: {phone_bodies:?}"
    );
    assert!(
        phone_bodies.contains(&"hello from phone, after pairing"),
        "the phone should still have its own message: {phone_bodies:?}"
    );

    // The desktop should also have the phone's message: a sync request
    // bundles the requester's own delta, so the responder converges too.
    let desktop_messages = desktop
        .list_messages(&carol_id)
        .await
        .expect("desktop lists its history with carol after answering the sync request");
    let desktop_bodies: Vec<&str> = desktop_messages.iter().map(|m| m.body.as_str()).collect();
    assert!(
        desktop_bodies.contains(&"hello from phone, after pairing"),
        "the desktop should have picked up the phone's message from the sync request's bundled delta: {desktop_bodies:?}"
    );

    // A second sync is a no-op: idempotent, no duplicate rows (`message_id`'s
    // unique index + `INSERT OR IGNORE`), and still completes cleanly.
    phone
        .sync_with_device(&alice_desktop_device_id)
        .await
        .expect("a second sync still succeeds");
    wait_for_sync_completed(&mut phone).await;
    let phone_messages_after_second_sync = phone
        .list_messages(&carol_id)
        .await
        .expect("phone lists its history after a second, redundant sync");
    assert_eq!(
        phone_messages_after_second_sync.len(),
        phone_messages.len(),
        "a second sync shouldn't duplicate any messages"
    );
}

/// Regression test: pairing's bootstrap snapshot only carries *contacts*,
/// never groups, so a group created *after* pairing was never discovered
/// on the phone until `sync_with_device` also re-ran `discover_missing_groups`.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn sync_recovers_a_group_created_after_pairing_already_completed() {
    let directory_url = spawn_directory_server().await;

    let alice_desktop_dir = tempfile::tempdir().unwrap();
    let alice_phone_dir = tempfile::tempdir().unwrap();

    let mut alice_desktop_svc = AppService::load_or_create(
        alice_desktop_dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("alice's desktop starts up");
    let alice_desktop_device_id = alice_desktop_svc.node.device_id().to_string();

    // No group exists yet — the phone pairs against an account that, at
    // this exact moment, genuinely has nothing to discover.
    let offer = alice_desktop_svc
        .start_pairing()
        .expect("alice starts a pairing offer");

    let desktop = spawn_node(alice_desktop_svc);

    let phone_svc = AppService::join_via_pairing(
        alice_phone_dir.path().to_path_buf(),
        directory_url.clone(),
        &offer,
    )
    .await
    .expect("alice's phone joins via pairing");
    let mut phone = spawn_node(phone_svc);

    // *Now* alice creates a group on her desktop — after the phone's own
    // one-shot startup discovery already ran (as part of the
    // `load_or_create` inside `join_via_pairing` above) and found nothing,
    // since there was nothing yet to find.
    let group = desktop
        .create_group("Weekend plans")
        .await
        .expect("alice's desktop creates a group after pairing");
    let channel_id = group.channels[0].channel_id.clone();
    let group_id = group.group_id.clone();

    // Confirms the actual gap: the phone has no restart-triggered moment
    // left to discover a group created after it already finished pairing.
    let phone_groups_before_sync = phone
        .list_groups()
        .await
        .expect("phone lists its (empty) groups before syncing");
    assert!(
        phone_groups_before_sync.is_empty(),
        "the phone shouldn't know about a group created after it paired, until it syncs: {phone_groups_before_sync:?}"
    );

    // Press "Sync" on the phone.
    phone
        .sync_with_device(&alice_desktop_device_id)
        .await
        .expect("phone requests a sync with the desktop");
    wait_for_sync_completed(&mut phone).await;

    // Group key recovery is a separate async exchange from the message
    // sync above with no ordering guarantee, so poll instead of assuming.
    let recovered_groups = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let groups = phone.list_groups().await.expect("phone lists its groups");
            if groups.iter().any(|g| g.group_id == group_id) {
                return groups;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("timed out waiting for the phone to recover the group via sync");
    let recovered_group = recovered_groups
        .iter()
        .find(|g| g.group_id == group_id)
        .expect("just checked it's there");
    assert_eq!(recovered_group.name, "Weekend plans");

    // Roster metadata alone isn't proof the Megolm key arrived — confirm
    // the phone can really decrypt a brand new message sent live.
    desktop
        .send_group_message(&group_id, &channel_id, "can you see this on your phone?")
        .await
        .expect("desktop sends a fresh group message");

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(event) = phone.event_rx.recv().await
                && let ChatEvent::GroupMessage {
                    group_id: gid,
                    body,
                    ..
                } = event
                && gid == group_id
            {
                assert_eq!(body, "can you see this on your phone?");
                return;
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "phone never received/decrypted the post-sync group message \
         — the Megolm key never actually arrived"
    );
}
