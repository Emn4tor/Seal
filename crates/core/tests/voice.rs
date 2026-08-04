use std::str::FromStr;
use std::time::Duration;

use identity::Identity;
use libp2p::Multiaddr;
use libp2p::core::multiaddr::Protocol;
use p2p_core::{ChatNode, VoiceCallState};

/// Two real `ChatNode`s, connected over loopback TCP, each start a
/// `VoiceCallState` for the same channel and one dials the other's raw
/// voice stream via real `libp2p-stream` negotiation — independent of any
/// real audio hardware (audio I/O gracefully degrades to a no-op on a
/// machine/test-runner with no usable mic/speaker; see
/// `spawn_audio_io_thread`). Presence bookkeeping is exercised as pure
/// local state, since gossip-driven discovery lives one layer up in
/// `AppService`.
// Ignored by default: `VoiceCallState::start` opens the real default
// mic/speaker as a side effect of exercising the network/presence logic
// this test actually cares about (it works fine either way — audio I/O
// degrades gracefully with no device/permission — but silently engaging
// someone's microphone on a routine `cargo test --workspace` run is a
// surprise this project shouldn't spring on anyone). Run explicitly with
// `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "opens the real default mic/speaker; run explicitly with `cargo test -- --ignored`"]
async fn two_nodes_establish_a_voice_call_and_track_presence() {
    let mut node_a = ChatNode::new(Identity::generate()).expect("build node a");
    let mut node_b = ChatNode::new(Identity::generate()).expect("build node b");

    let peer_a = node_a.local_peer_id();
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

    let call_a = VoiceCallState::start("group-1".into(), "channel-1".into(), control_a)
        .await
        .expect("start call on a");
    let call_b = VoiceCallState::start("group-1".into(), "channel-1".into(), control_b)
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
