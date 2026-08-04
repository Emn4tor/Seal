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
    GossipSubscribed {
        peer_id: PeerId,
        topic: String,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    /// We appear to be reachable by a direct dial from the outside.
    Public,
    /// We're behind a NAT/firewall that direct dials can't reach — relay +
    /// hole-punching (dcutr) is what makes us reachable at all.
    Private,
    /// Not enough probes yet to know either way.
    Unknown,
}
