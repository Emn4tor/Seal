use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::AppService;
use tokio::sync::{mpsc, oneshot};

/// The other integration tests in this crate drive both sides from a
/// *single* task's `select!` loop, calling `send_direct_message` etc.
/// synchronously between polls. That's not what the real app does —
/// `apps/desktop/src-tauri/src/actor.rs` gives each side its own
/// independently-scheduled background task that's *always* polling
/// `next_event()` and only handles a command when one happens to be ready,
/// via an mpsc channel exactly like this file reproduces. This exists
/// because that structural difference is exactly where a real bug was
/// once hiding (see `contact_reachability_after_restart.rs`) — actor.rs
/// itself otherwise has zero test coverage.
enum Cmd {
    AddContact(String, oneshot::Sender<anyhow::Result<()>>),
    SendDm(String, String, oneshot::Sender<anyhow::Result<()>>),
    CreateGroup(String, oneshot::Sender<anyhow::Result<p2p_core::GroupInfo>>),
    InviteToGroup(
        String,
        String,
        oneshot::Sender<anyhow::Result<p2p_core::GroupInfo>>,
    ),
    SendGroupMsg(String, String, String, oneshot::Sender<anyhow::Result<()>>),
    ListMessages(
        String,
        oneshot::Sender<anyhow::Result<Vec<storage::StoredMessage>>>,
    ),
    ListGroups(oneshot::Sender<anyhow::Result<Vec<storage::StoredGroup>>>),
}

struct Handle {
    tx: mpsc::Sender<Cmd>,
}

impl Handle {
    async fn add_contact(&self, id: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::AddContact(id.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    async fn send_dm(&self, peer: &str, body: &str) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::SendDm(peer.to_string(), body.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    async fn create_group(&self, name: &str) -> anyhow::Result<p2p_core::GroupInfo> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::CreateGroup(name.to_string(), tx))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    async fn invite(&self, group_id: &str, member: &str) -> anyhow::Result<p2p_core::GroupInfo> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::InviteToGroup(
                group_id.to_string(),
                member.to_string(),
                tx,
            ))
            .await
            .unwrap();
        rx.await.unwrap()
    }
    async fn send_group_msg(
        &self,
        group_id: &str,
        channel_id: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::SendGroupMsg(
                group_id.to_string(),
                channel_id.to_string(),
                body.to_string(),
                tx,
            ))
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
    async fn list_groups(&self) -> anyhow::Result<Vec<storage::StoredGroup>> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Cmd::ListGroups(tx)).await.unwrap();
        rx.await.unwrap()
    }
}

/// Spawns a background task that owns `svc` outright and, exactly like
/// `actor.rs::run`, alternates between handling a queued command and
/// draining whatever `next_event()` produces — never blocking one on the
/// other beyond what `select!` naturally does.
fn spawn_actor(mut svc: AppService) -> Handle {
    let (tx, mut rx) = mpsc::channel::<Cmd>(32);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                maybe_cmd = rx.recv() => {
                    let Some(cmd) = maybe_cmd else { return };
                    match cmd {
                        Cmd::AddContact(id, respond) => {
                            let _ = respond.send(svc.add_contact_by_user_id(&id).await);
                        }
                        Cmd::SendDm(peer, body, respond) => {
                            let _ = respond.send(svc.send_direct_message(&peer, &body, None).await);
                        }
                        Cmd::CreateGroup(name, respond) => {
                            let _ = respond.send(svc.create_group(&name).await);
                        }
                        Cmd::InviteToGroup(group_id, member, respond) => {
                            let _ = respond.send(svc.invite_to_group(&group_id, &member).await);
                        }
                        Cmd::SendGroupMsg(group_id, channel_id, body, respond) => {
                            let _ = respond.send(
                                svc.send_group_message(&group_id, &channel_id, &body, None).await,
                            );
                        }
                        Cmd::ListMessages(conv, respond) => {
                            let _ = respond.send(svc.list_messages(&conv));
                        }
                        Cmd::ListGroups(respond) => {
                            let _ = respond.send(svc.list_groups());
                        }
                    }
                }
                event = svc.next_event() => {
                    // Auto-accept/self-heal side effects (group invites,
                    // unknown-sender contacts) happen inside `next_event`
                    // itself; this loop just needs to keep draining it so
                    // the swarm makes progress, same as `actor.rs::run`.
                    let _ = event;
                }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_independent_actor_loops_dm_and_group_like_the_real_app() {
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
    let bob = spawn_actor(bob_svc);

    // Mimic real user timing: pauses between actions, exactly like a human
    // clicking through the UI, rather than firing everything back-to-back.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Alice adds bob as a contact; bob never explicitly adds alice back —
    // he discovers her purely through the self-heal path on an unknown
    // sender (`AppService::next_event`), matching the common real-world
    // flow of "only one side clicks Add".
    alice.add_contact(&bob_id).await.expect("alice adds bob");
    tokio::time::sleep(Duration::from_millis(200)).await;

    alice
        .send_dm(&bob_id, "hello bob")
        .await
        .expect("alice sends dm");
    tokio::time::sleep(Duration::from_secs(2)).await;

    let bob_history = bob
        .list_messages(&alice_id)
        .await
        .expect("bob's dm history");
    assert_eq!(
        bob_history.len(),
        1,
        "bob should have alice's DM in his local store"
    );
    assert_eq!(bob_history[0].body, "hello bob");

    bob.send_dm(&alice_id, "hi alice")
        .await
        .expect("bob replies");
    tokio::time::sleep(Duration::from_secs(2)).await;

    let alice_history = alice
        .list_messages(&bob_id)
        .await
        .expect("alice's dm history");
    assert_eq!(
        alice_history.len(),
        2,
        "alice should see her own sent message plus bob's reply"
    );

    let group = alice
        .create_group("test group")
        .await
        .expect("alice creates group");
    let group_id = group.group_id.clone();
    let channel_id = group.channels[0].channel_id.clone();

    let updated = alice
        .invite(&group_id, &bob_id)
        .await
        .expect("alice invites bob");
    assert_eq!(updated.members.len(), 2, "alice should see 2 members");

    tokio::time::sleep(Duration::from_secs(2)).await;

    let bob_groups = bob.list_groups().await.expect("bob's groups");
    assert_eq!(
        bob_groups.len(),
        1,
        "bob should have the group stored locally after accepting the invite"
    );
    assert_eq!(
        bob_groups[0].members.len(),
        2,
        "bob's copy should also show 2 members"
    );

    alice
        .send_group_msg(&group_id, &channel_id, "hello group")
        .await
        .expect("alice sends group message");
    tokio::time::sleep(Duration::from_secs(2)).await;

    let bob_group_history = bob
        .list_messages(&format!("{group_id}:{channel_id}"))
        .await
        .expect("bob's group message history");
    assert_eq!(
        bob_group_history.len(),
        1,
        "bob should have received the group message"
    );
}
