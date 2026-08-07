use std::str::FromStr;
use std::time::Duration;

use identity::Identity;
use libp2p::Multiaddr;
use libp2p::core::multiaddr::Protocol;
use p2p_core::{CallScope, ChatNode, VoiceCallState};

/// Two real `ChatNode`s, connected over loopback TCP, each start a
/// `VoiceCallState` for the same channel and one dials the other's raw
/// voice stream via real `libp2p-stream` negotiation — independent of
/// real audio hardware (see `spawn_audio_io_thread`). Presence
/// bookkeeping is exercised as pure local state; gossip-driven discovery
/// lives one layer up in `AppService`.
// Ignored by default: `VoiceCallState::start` opens the real default
// mic/speaker as a side effect (works fine either way — audio I/O
// degrades gracefully with no device — but silently engaging someone's
// microphone on a routine `cargo test --workspace` run is a surprise
// this project shouldn't spring on anyone). Run explicitly with
// `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "opens the real default mic/speaker; run explicitly with `cargo test -- --ignored`"]
async fn two_nodes_establish_a_voice_call_and_track_presence() {
    let mut node_a = ChatNode::new(Identity::generate()).expect("build node a");
    let mut node_b = ChatNode::new(Identity::generate()).expect("build node b");

    let peer_a = node_a.local_peer_id();
    let peer_b = node_b.local_peer_id();
    let control_a = node_a.voice_control();
    let control_b = node_b.voice_control();

    node_a
        .listen_on(Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap())
        .expect("listen on a");
    let addr_a = node_a.wait_for_listen_addr().await;

    node_b
        .dial(addr_a.with(Protocol::P2p(peer_a)))
        .expect("dial a from b");

    tokio::spawn(async move {
        loop {
            node_a.next_event().await;
        }
    });
    tokio::spawn(async move {
        loop {
            node_b.next_event().await;
        }
    });

    let scope = || CallScope::Group {
        group_id: "group-1".into(),
        channel_id: "channel-1".into(),
    };
    let call_a = VoiceCallState::start(scope(), control_a, None, None)
        .await
        .expect("start call on a");
    let call_b = VoiceCallState::start(scope(), control_b, None, None)
        .await
        .expect("start call on b");

    // Pure local bookkeeping — no network activity involved.
    assert!(call_a.note_presence("bob", true));
    assert_eq!(call_a.participants(), vec!["bob".to_string()]);
    assert!(
        !call_a.note_presence("bob", true),
        "re-announcing an existing participant is a no-op"
    );
    assert!(call_a.note_presence("bob", false));
    assert!(call_a.participants().is_empty());

    // The acceptor only registers an inbound stream from a peer it already
    // knows is an expected participant (see `resolve_authorized_participant`
    // in `voice.rs`). A real call always learns this via `connect_voice_peer`
    // before dialing, so the test has to set up the same two facts by hand.
    call_a.note_presence("alice", true);
    call_a.note_peer_identity(peer_b, "alice".into());

    // Real mesh dial + libp2p-stream negotiation between two live swarms.
    tokio::time::timeout(
        Duration::from_secs(15),
        call_b.ensure_connected(peer_a, "alice".into()),
    )
    .await
    .expect("timed out opening a voice stream")
    .expect("failed to open a voice stream from b to a");

    // A second call is a no-op (already connected), not a duplicate stream.
    call_b
        .ensure_connected(peer_a, "alice".into())
        .await
        .expect("ensure_connected should be idempotent");

    // Give the acceptor side a moment to register the inbound connection
    // before tearing everything down.
    tokio::time::sleep(Duration::from_millis(200)).await;

    drop(call_a);
    drop(call_b);
}
