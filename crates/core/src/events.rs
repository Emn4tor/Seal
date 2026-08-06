use crypto_session::AttachmentPayload;
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub enum ChatEvent {
    Connected(PeerId),
    DirectMessage {
        from: String,
        body: String,
        attachment: Option<AttachmentPayload>,
    },
    GroupMessage {
        group_id: String,
        channel_id: String,
        from: String,
        body: String,
        attachment: Option<AttachmentPayload>,
    },
    /// A group session key was received and registered — we can now decrypt
    /// that sender's future messages in this group. Useful both for tests
    /// (sequencing) and for UI (e.g. clearing an "unable to decrypt yet"
    /// placeholder).
    GroupKeyReceived {
        group_id: String,
        from: String,
    },
    /// A fellow member asked us for a group's key — see `DirectPayload::
    /// GroupKeyRequest`'s doc comment for why this exists. Purely an
    /// internal protocol handshake: `AppService::next_event` handles it by
    /// re-sharing (after verifying roster membership) and never returns
    /// it, so it has no frontend-facing shape and callers outside
    /// `p2p_core` should never see it.
    GroupKeyRequested {
        group_id: String,
        from: String,
    },
    GossipSubscribed {
        peer_id: PeerId,
        topic: String,
    },
    /// A direct message or group-key-share never reached the peer (dial
    /// failure, timeout, connection reset, …). Previously this just
    /// vanished silently — sending looked identical whether or not it
    /// actually worked. `peer_user_id` is `None` when the failure came from
    /// a peer id we don't have a contact for (shouldn't normally happen,
    /// since we only ever send to peer ids resolved from a contact).
    MessageSendFailed {
        peer_user_id: Option<String>,
        reason: String,
    },
    /// AutoNAT's assessment of whether we're publicly reachable changed —
    /// surfaced so the UI can show something more honest than a silent
    /// spinner about why a peer might only be reachable via relay.
    NetworkStatus(NetworkStatus),
    /// Raw signal decrypted off a group's gossipsub topic: one member
    /// announced joining or leaving a voice channel. This is intercepted by
    /// `AppService::next_event` (which owns the directory lookups needed to
    /// resolve and dial the announcer) and translated into
    /// `VoiceParticipantsChanged` for anything actually in that call —
    /// callers outside `p2p_core` should never see this variant directly.
    VoicePresence {
        group_id: String,
        channel_id: String,
        from: String,
        joined: bool,
    },
    /// The known participant set for a voice channel we're currently in
    /// changed — presence-driven (see `VoicePresence`), independent of
    /// whether a `libp2p-stream` connection to each participant has
    /// actually succeeded yet.
    VoiceParticipantsChanged {
        group_id: String,
        channel_id: String,
        user_ids: Vec<String>,
    },
    /// A fellow member created a new channel in a group we're in.
    /// Intercepted by `AppService::next_event` (which owns the directory
    /// lookup needed to refetch the group), same shape as `GroupKeyReceived`
    /// — callers outside `p2p_core` see the frontend-facing effect of this
    /// (an updated channel list) rather than this variant directly.
    GroupChannelsChanged {
        group_id: String,
    },
    /// Someone is calling us 1:1 — the frontend should show an incoming-call
    /// screen (ring sound, accept/decline). `AppService::next_event` already
    /// auto-declines this (sending `CallDecline` back) if we're already in
    /// or ringing another call, so by the time the frontend sees this, it's
    /// always safe to actually offer.
    CallInvited {
        from: String,
        call_id: String,
    },
    /// The person we called picked up — the caller's cue to move from
    /// "ringing" to the active call UI. The actual voice stream connects
    /// separately (see `AppService::accept_call`); this just reports the
    /// signaling outcome.
    CallAccepted {
        from: String,
        call_id: String,
    },
    /// The person we called rejected it.
    CallDeclined {
        from: String,
        call_id: String,
    },
    /// The other side ended the call — cancelled it while it was still
    /// ringing, or hung up an active one. Either way, our own local call
    /// state for `call_id` is already torn down by the time this is
    /// returned; the frontend just needs to leave the call UI.
    CallEnded {
        from: String,
        call_id: String,
    },
    /// Our own `CallInvite` (ringing someone) or `CallAccept` (answering
    /// someone) never reached them — most commonly because their
    /// last-known address is stale and they're not actually online right
    /// now, but also a transport-level dial failure/timeout, or the
    /// recipient's side failing to decrypt it. Distinct from the generic
    /// `MessageSendFailed` (see `ChatNode::pending_call_sends`) so the
    /// frontend can tell "the call failed" apart from "an unrelated
    /// message to this same person failed" and react precisely — cancel
    /// the ring screen and say why, rather than leave it hanging or show
    /// an unrelated toast. Our own local call state for `call_id` is
    /// already torn down by the time this is returned.
    CallFailed {
        peer_user_id: String,
        call_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    /// We appear to be reachable by a direct dial from the outside.
    Public,
    /// We're behind a NAT/firewall that direct dials can't reach: relay +
    /// hole-punching (dcutr) is what makes us reachable at all.
    Private,
    /// Not enough probes yet to know either way.
    Unknown,
}
