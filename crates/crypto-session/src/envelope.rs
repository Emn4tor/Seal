use serde::{Deserialize, Serialize};

/// The outer wire envelope for a 1:1 direct message, carried as the opaque
/// `payload` bytes of `net::ChatRequest`.
///
/// `sender_user_id`/`sender_curve25519_key` are unauthenticated *routing
/// hints* — they tell the receiver which cached contact/session to use, not
/// a security claim. The actual authentication is the Olm decryption itself:
/// it only succeeds against the session bound to the peer's real Curve25519
/// identity key during the 3DH handshake. A lie here just causes decryption
/// to fail (wrong or missing session); it can never make forged plaintext
/// look like it came from someone else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectEnvelope {
    pub sender_user_id: String,
    pub sender_curve25519_key: String,
    /// Mirrors `vodozemac::olm::OlmMessage::to_parts()` — 0 for a pre-key
    /// (session-establishing) message, otherwise a normal message.
    pub message_type: u8,
    pub ciphertext: Vec<u8>,
}

/// A file/image attached to a chat message. Carried inside `DirectPayload`
/// and `GroupPayload::Chat` — encrypted (and, for groups, key-rotated) by
/// exactly the same Olm/Megolm sessions as a plain text body, never treated
/// as a second, less-protected channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentPayload {
    pub filename: String,
    pub mime_type: String,
    /// Whether EXIF metadata was stripped from `data` before it was sent —
    /// surfaced to the recipient so they know whether to trust/inspect it
    /// (see the "view EXIF" UI for the `false` case).
    pub exif_stripped: bool,
    pub data: Vec<u8>,
}

/// The plaintext carried *inside* Olm encryption for a 1:1 exchange. Using
/// the same 1:1 transport for both actual chat messages and group-key
/// distribution keeps the wire format small and reuses the exact same
/// authenticated, forward-secret channel for both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectPayload {
    Chat {
        body: String,
        #[serde(default)]
        attachment: Option<AttachmentPayload>,
    },
    GroupKeyShare {
        group_id: String,
        session_key_bytes: Vec<u8>,
    },
}

/// The wire envelope for a group message, published to the group's gossipsub
/// topic. Megolm's own signature (over `ciphertext`) authenticates "which
/// chat identity's session encrypted this"; gossipsub's own message signing
/// (by the libp2p transport key) is a separate, complementary guarantee
/// about which network node published it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupEnvelope {
    pub group_id: String,
    pub sender_user_id: String,
    pub session_id: String,
    pub ciphertext: Vec<u8>,
}

/// The plaintext carried inside a group's Megolm encryption. `channel_id` is
/// a routing field only, not a new crypto/topic dimension — every channel in
/// a group is visible to every group member in this phase (no per-channel
/// permissions), so channels deliberately reuse the group's one shared
/// Megolm session and gossipsub topic rather than each needing an
/// independent one. See `crates/core/src/node.rs`'s `send_group_message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupPayload {
    Chat {
        channel_id: String,
        body: String,
        #[serde(default)]
        attachment: Option<AttachmentPayload>,
    },
    /// Re-broadcast on a ~5s heartbeat by each participant while a voice
    /// channel call is active. Gossipsub has no history/replay, so this is
    /// how a member who joins a channel after others are already in a call
    /// discovers who else is present — there's no other way to learn that
    /// short of asking. `joined: false` announces a deliberate leave rather
    /// than making everyone else wait for the heartbeat to simply stop.
    VoicePresence { channel_id: String, joined: bool },
}
