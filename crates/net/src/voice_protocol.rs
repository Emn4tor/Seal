//! Protocol id and thin helpers for the raw voice-audio stream — a plain
//! `AsyncRead + AsyncWrite` per peer, opened on demand while a voice
//! channel call is active. Callers get `Control` from
//! `swarm.behaviour().stream.new_control()`; everything here just bakes in
//! the protocol id so `crates/core` doesn't need to depend on
//! `libp2p_stream` or repeat the `StreamProtocol` construction.

use libp2p::{PeerId, Stream, StreamProtocol};
use libp2p_stream::{AlreadyRegistered, Control, IncomingStreams, OpenStreamError};

pub const VOICE_PROTOCOL: &str = "/p2p-chat/voice/1";

pub fn voice_protocol() -> StreamProtocol {
    StreamProtocol::new(VOICE_PROTOCOL)
}

/// Opens an outbound voice stream to `peer`, dialing them if not already
/// connected.
pub async fn open_voice_stream(control: &mut Control, peer: PeerId) -> Result<Stream, OpenStreamError> {
    control.open_stream(peer, voice_protocol()).await
}

/// Starts accepting inbound voice streams. Drop the returned handle to stop
/// (e.g. when leaving the voice channel) — any further inbound attempt then
/// fails negotiation on the remote's side.
pub fn accept_voice_streams(control: &mut Control) -> Result<IncomingStreams, AlreadyRegistered> {
    control.accept(voice_protocol())
}
