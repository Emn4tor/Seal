use libp2p::identity::Keypair;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{StreamProtocol, autonat, dcutr, gossipsub, identify, ping, relay, request_response};

use crate::protocol::{ChatRequest, ChatResponse, DIRECT_PROTOCOL};

/// Purely cosmetic version string exchanged by the `identify` protocol —
/// not a security boundary, just diagnostic metadata about which app is
/// speaking on the other end of a connection.
const IDENTIFY_PROTOCOL_VERSION: &str = "/p2p-chat/id/1";

/// Both defaults (1 MiB request / 10 MiB response for request_response;
/// 64 KiB for gossipsub) are sized for text messages, not attachments —
/// raised here to comfortably fit `p2p_core::MAX_ATTACHMENT_SIZE` (25 MB)
/// plus envelope/ciphertext overhead.
const MAX_TRANSPORT_MESSAGE_SIZE: usize = 30 * 1024 * 1024;

#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: dcutr::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
    pub request_response: request_response::cbor::Behaviour<ChatRequest, ChatResponse>,
    pub autonat: autonat::Behaviour,
    /// Raw per-peer duplex byte streams for continuous voice audio — neither
    /// `request_response` (discrete request->response) nor `gossipsub`
    /// (broadcast) fit a call. See `voice_protocol` for the protocol id and
    /// helpers around this behaviour's `Control`.
    pub stream: libp2p_stream::Behaviour,
}

/// Builds the composed behaviour. `relay_client` comes from the swarm
/// builder's `with_relay_client(...)` step, which owns constructing the
/// matching relay transport — the behaviour half is threaded through here so
/// it ends up on the same swarm as everything else.
pub fn build_behaviour(keypair: &Keypair, relay_client: relay::client::Behaviour) -> ChatBehaviour {
    let local_peer_id = keypair.public().to_peer_id();

    let identify = identify::Behaviour::new(identify::Config::new(
        IDENTIFY_PROTOCOL_VERSION.to_string(),
        keypair.public(),
    ));

    // Signed authenticity: gossipsub itself authenticates "which libp2p node
    // published this" — a different, complementary guarantee from Megolm's
    // own per-message authentication of "which chat identity encrypted this"
    // that gets layered on top in Phase 4.
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .max_transmit_size(MAX_TRANSPORT_MESSAGE_SIZE)
        .build()
        .expect("static gossipsub config is valid");
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(keypair.clone()),
        gossipsub_config,
    )
    .expect("static gossipsub config is valid");

    let request_response_codec = request_response::cbor::codec::Codec::default()
        .set_request_size_maximum(MAX_TRANSPORT_MESSAGE_SIZE as u64)
        .set_response_size_maximum(MAX_TRANSPORT_MESSAGE_SIZE as u64);
    let request_response = request_response::cbor::Behaviour::with_codec(
        request_response_codec,
        [(
            StreamProtocol::new(DIRECT_PROTOCOL),
            request_response::ProtocolSupport::Full,
        )],
        request_response::Config::default(),
    );

    ChatBehaviour {
        identify,
        ping: ping::Behaviour::default(),
        relay_client,
        dcutr: dcutr::Behaviour::new(local_peer_id),
        gossipsub,
        request_response,
        autonat: autonat::Behaviour::new(local_peer_id, autonat::Config::default()),
        stream: libp2p_stream::Behaviour::new(),
    }
}
