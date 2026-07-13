use std::time::Duration;

use futures::io::{AsyncReadExt, AsyncWriteExt};
use futures::StreamExt;
use libp2p::core::multiaddr::Protocol;
use libp2p::swarm::SwarmEvent;
use net::{accept_voice_streams, build_swarm_with_new_identity, open_voice_stream};

/// One 20ms IMA ADPCM frame's worth of bytes (`audio::adpcm::BYTES_PER_FRAME`
/// — not depended on directly here to keep `net` decoupled from `audio`;
/// the exact size doesn't matter to this transport-layer test, only that
/// it's audio-frame-shaped).
const FRAME_LEN: usize = 160;

/// Two in-process swarms open a raw `libp2p-stream` voice stream (instead
/// of request_response/gossipsub, neither of which fit continuous audio)
/// and exchange one audio-shaped frame each way. Exercises the same
/// transport/relay-capable swarm as every other protocol in this crate,
/// just over the new stream behaviour.
#[tokio::test]
async fn two_swarms_exchange_audio_shaped_frames_over_a_voice_stream() {
    let mut swarm_a = build_swarm_with_new_identity().expect("build swarm a");
    let mut swarm_b = build_swarm_with_new_identity().expect("build swarm b");

    let peer_a = *swarm_a.local_peer_id();

    let mut control_a = swarm_a.behaviour().stream.new_control();
    let mut control_b = swarm_b.behaviour().stream.new_control();

    let mut incoming_a = accept_voice_streams(&mut control_a).expect("register accept on a");

    swarm_a
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("listen on a");
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    swarm_b
        .dial(addr_a.with(Protocol::P2p(peer_a)))
        .expect("dial a from b");

    // Stream negotiation needs each swarm's event loop actively polled;
    // once a Stream is handed back, its own I/O no longer depends on this.
    tokio::spawn(async move {
        loop {
            swarm_a.select_next_some().await;
        }
    });
    tokio::spawn(async move {
        loop {
            swarm_b.select_next_some().await;
        }
    });

    let echo_task = tokio::spawn(async move {
        let (_peer, mut stream) = tokio::time::timeout(Duration::from_secs(15), incoming_a.next())
            .await
            .expect("timed out waiting for inbound voice stream")
            .expect("incoming-streams channel closed unexpectedly");
        let mut buf = [0u8; FRAME_LEN];
        stream.read_exact(&mut buf).await.expect("read frame on a");
        stream.write_all(&buf).await.expect("echo frame back");
    });

    let sent_frame: [u8; FRAME_LEN] = std::array::from_fn(|i| i as u8);

    let mut stream_b = tokio::time::timeout(
        Duration::from_secs(15),
        open_voice_stream(&mut control_b, peer_a),
    )
    .await
    .expect("timed out opening voice stream")
    .expect("open voice stream from b to a");

    stream_b.write_all(&sent_frame).await.expect("write frame on b");

    let mut echoed = [0u8; FRAME_LEN];
    tokio::time::timeout(Duration::from_secs(15), stream_b.read_exact(&mut echoed))
        .await
        .expect("timed out reading echo")
        .expect("read echo on b");

    assert_eq!(echoed, sent_frame);

    echo_task.await.expect("echo task panicked");
}

async fn wait_for_listen_addr(swarm: &mut libp2p::Swarm<net::ChatBehaviour>) -> libp2p::Multiaddr {
    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}
