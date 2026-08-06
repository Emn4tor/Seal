use std::time::Duration;

use directory_server::{AppState, build_public_router};
use p2p_core::{AppService, AttachmentPayload, ChatEvent};
use serial_test::serial;

/// Spawns a real directory-server on loopback and returns its base URL. The
/// backing temp dir is intentionally leaked for the process's lifetime —
/// fine in a short-lived test process, and avoids plumbing a guard through
/// every caller.
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

/// This is the fullest-stack test in the project: two `AppService`s (the
/// exact type a Tauri command layer wraps) register with a real directory
/// server, discover each other purely by user_id, exchange an encrypted
/// DM, and run a group invite + group message, all persisted locally.
///
/// `#[serial]`, along with every other test here that creates a real
/// `AppService`: each resolves its OS-keychain KEK, and libtest runs test
/// functions concurrently by default, which on Windows was enough to
/// occasionally corrupt a concurrently-accessed identity blob (a
/// Credential Manager race, not a logic bug — see `storage::crypto`'s own
/// unit tests). Serializing these tests relative to each other removes
/// that race.
///
/// Also ignored on Linux, along with every other test here: `Keychain`
/// needs a real D-Bus Secret Service, which headless CI doesn't have, and
/// `gnome-keyring-daemon` proved too unreliable to depend on. Same
/// disclosed-gap treatment as this project's real-audio-hardware tests;
/// real coverage still runs on macOS/Windows.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn full_app_service_flow_dm_then_group() {
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

    let dm_attachment = AttachmentPayload {
        filename: "hello.txt".to_string(),
        mime_type: "text/plain".to_string(),
        exif_stripped: false,
        data: b"attachment bytes for the dm".to_vec(),
    };
    alice
        .send_direct_message(&bob_id, "hello bob", Some(dm_attachment))
        .await
        .expect("alice sends dm");

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                _ = alice.next_event() => {}
                event = bob.next_event() => {
                    if let ChatEvent::DirectMessage { from, body, attachment } = event {
                        assert_eq!(from, alice_id);
                        assert_eq!(body, "hello bob");
                        assert_eq!(
                            attachment.expect("dm had an attachment").data,
                            b"attachment bytes for the dm"
                        );
                        break;
                    }
                }
            }
        }
    })
    .await;
    assert!(result.is_ok(), "timed out waiting for the DM to arrive");

    let alice_history = alice.list_messages(&bob_id).expect("alice's dm history");
    assert_eq!(alice_history.len(), 1);
    assert_eq!(alice_history[0].body, "hello bob");
    assert_eq!(
        alice_history[0]
            .attachment
            .as_ref()
            .expect("alice's own stored copy has the attachment too")
            .filename,
        "hello.txt"
    );
    let bob_history = bob.list_messages(&alice_id).expect("bob's dm history");
    assert_eq!(bob_history.len(), 1);
    assert_eq!(bob_history[0].sender_user_id, alice_id);
    assert_eq!(
        bob_history[0]
            .attachment
            .as_ref()
            .expect("bob stored the attachment")
            .data,
        b"attachment bytes for the dm"
    );

    // Group flow: alice creates a group and invites bob.
    let group = alice
        .create_group("test group")
        .await
        .expect("alice creates group");
    alice
        .invite_to_group(&group.group_id, &bob_id)
        .await
        .expect("alice invites bob");

    let group_id = group.group_id.clone();
    // Every group starts with a default "general" text channel — no
    // separate create-channel call needed just to have somewhere to send.
    assert_eq!(group.channels.len(), 1);
    assert_eq!(group.channels[0].name, "general");
    let channel_id = group.channels[0].channel_id.clone();
    let mut bob_accepted = false;
    let mut bob_gossip_subscribed = false;
    let mut bob_got_group_message = false;
    let mut sent_group_message = false;

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if bob_got_group_message {
                break;
            }
            tokio::select! {
                event = alice.next_event() => {
                    if let ChatEvent::GossipSubscribed { .. } = event {
                        bob_gossip_subscribed = true;
                    }
                }
                event = bob.next_event() => {
                    if let ChatEvent::GroupKeyReceived { group_id: gid, from } = &event {
                        assert_eq!(*gid, group_id);
                        assert_eq!(*from, alice_id);
                        bob.accept_group_invite(&group_id)
                            .await
                            .expect("bob accepts group invite");
                        bob_accepted = true;
                    }
                    if let ChatEvent::GroupMessage { from, channel_id: cid, body, .. } = event {
                        assert_eq!(from, alice_id);
                        assert_eq!(cid, channel_id);
                        assert_eq!(body, "hello group");
                        bob_got_group_message = true;
                    }
                }
            }

            // Wait for bob's subscription to be visible on alice's gossipsub
            // (not just for bob to have locally called subscribe) before
            // publishing, or gossipsub has nobody to fan the message out to
            // yet.
            if bob_accepted && bob_gossip_subscribed && !sent_group_message {
                sent_group_message = true;
                alice
                    .send_group_message(&group_id, &channel_id, "hello group", None)
                    .await
                    .expect("alice sends group message");
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for the group message to arrive"
    );

    let conversation_id = format!("{group_id}:{channel_id}");
    let bob_group_history = bob
        .list_messages(&conversation_id)
        .expect("bob's group history");
    assert_eq!(bob_group_history.len(), 1);
    assert_eq!(bob_group_history[0].body, "hello group");

    let bob_groups = bob.list_groups().expect("bob's groups");
    assert_eq!(bob_groups.len(), 1);
    assert_eq!(bob_groups[0].members.len(), 2);
    assert_eq!(bob_groups[0].channels.len(), 1);
    assert_eq!(bob_groups[0].channels[0].name, "general");
}

/// Regression test: re-loading an existing identity used to always use
/// whatever `display_name` the caller passed in, silently renaming the
/// account on every restart. The stored name must win instead.
///
/// `#[serial]`/`#[cfg_attr(... ignore)]`: see
/// `full_app_service_flow_dm_then_group`'s doc comment.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn resuming_an_account_keeps_its_stored_name_not_the_caller_supplied_one() {
    let directory_url = spawn_directory_server().await;
    let dir = tempfile::tempdir().unwrap();

    let alice = AppService::load_or_create(
        dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("first launch creates the account");
    assert_eq!(alice.display_name(), "alice");
    let user_id = alice.user_id();
    drop(alice);

    // A second "launch" against the same data dir, as if the caller had
    // (wrongly) passed a different name along — this must be ignored.
    let resumed = AppService::load_or_create(
        dir.path().to_path_buf(),
        directory_url,
        Some("someone-else".to_string()),
    )
    .await
    .expect("second launch resumes the existing account");
    assert_eq!(resumed.display_name(), "alice");
    assert_eq!(resumed.user_id(), user_id);
}

/// A genuinely new account with no display name at all should fail clearly
/// rather than e.g. registering with an empty name.
///
/// `#[serial]`/`#[cfg_attr(... ignore)]`: see
/// `full_app_service_flow_dm_then_group`'s doc comment.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn creating_a_new_account_without_a_name_is_an_error() {
    let directory_url = spawn_directory_server().await;
    let dir = tempfile::tempdir().unwrap();

    let result = AppService::load_or_create(dir.path().to_path_buf(), directory_url, None).await;
    assert!(result.is_err());
}

/// `rename` is the one deliberate path a display name should change through:
/// it must update what a later `load_or_create` resumes with.
///
/// `#[serial]`: see `full_app_service_flow_dm_then_group`'s doc comment —
/// this is the specific test that surfaced the Windows race.
/// `#[cfg_attr(... ignore)]`: same doc comment, the Linux side of it.
#[tokio::test]
#[serial]
#[cfg_attr(
    target_os = "linux",
    ignore = "needs a real D-Bus Secret Service, not reliably available on headless CI — see this comment"
)]
async fn rename_persists_and_survives_resume() {
    let directory_url = spawn_directory_server().await;
    let dir = tempfile::tempdir().unwrap();

    let mut alice = AppService::load_or_create(
        dir.path().to_path_buf(),
        directory_url.clone(),
        Some("alice".to_string()),
    )
    .await
    .expect("first launch creates the account");

    alice
        .rename("alice-renamed".to_string())
        .await
        .expect("rename succeeds");
    assert_eq!(alice.display_name(), "alice-renamed");
    let user_id = alice.user_id();
    drop(alice);

    let resumed = AppService::load_or_create(dir.path().to_path_buf(), directory_url, None)
        .await
        .expect("resumes after rename");
    assert_eq!(resumed.display_name(), "alice-renamed");
    assert_eq!(resumed.user_id(), user_id);
}
