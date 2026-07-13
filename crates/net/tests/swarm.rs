use std::time::Duration;

use futures::StreamExt;
use libp2p::core::multiaddr::Protocol;
use libp2p::request_response;
use libp2p::swarm::SwarmEvent;
use net::{ChatBehaviourEvent, ChatRequest, ChatResponse, build_swarm_with_new_identity};

/// Two in-process swarms, connected over loopback TCP, exchange a single
/// request/response pair on our custom direct-message protocol. This
/// exercises the composed `ChatBehaviour` end to end at the transport layer,
/// independent of any relay/NAT-traversal machinery (which is a no-op here
/// since both peers are directly dialable).
#[tokio::test]
async fn two_swarms_exchange_a_request_response_message() {
    let mut swarm_a = build_swarm_with_new_identity().expect("build swarm a");
    let mut swarm_b = build_swarm_with_new_identity().expect("build swarm b");

    let peer_a = *swarm_a.local_peer_id();
    let peer_b = *swarm_b.local_peer_id();

    swarm_a
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .expect("listen on a");

    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let dial_addr_a = addr_a.clone().with(Protocol::P2p(peer_a));
    swarm_b.dial(dial_addr_a).expect("dial a from b");

    let mut request_sent = false;
    let mut received_ack = false;

    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if received_ack {
                break;
            }
            tokio::select! {
                event = swarm_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(ChatBehaviourEvent::RequestResponse(
                        request_response::Event::Message {
                            message: request_response::Message::Request { request, channel, .. },
                            ..
                        },
                    )) = event
                    {
                        assert_eq!(request.payload, b"hello from b");
                        swarm_a
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, ChatResponse { ack: true })
                            .expect("send response");
                    }
                }
                event = swarm_b.select_next_some() => {
                    if let SwarmEvent::ConnectionEstablished { peer_id, .. } = &event
                        && *peer_id == peer_a
                        && !request_sent
                    {
                        request_sent = true;
                        swarm_b.behaviour_mut().request_response.send_request(
                            &peer_a,
                            ChatRequest { payload: b"hello from b".to_vec() },
                        );
                    }
                    if let SwarmEvent::Behaviour(ChatBehaviourEvent::RequestResponse(
                        request_response::Event::Message {
                            message: request_response::Message::Response { response, .. },
                            ..
                        },
                    )) = event
                    {
                        assert!(response.ack);
                        received_ack = true;
                    }
                }
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "timed out before receiving an ack; peer_a={peer_a}, peer_b={peer_b}"
    );
}

async fn wait_for_listen_addr(swarm: &mut libp2p::Swarm<net::ChatBehaviour>) -> libp2p::Multiaddr {
    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}
