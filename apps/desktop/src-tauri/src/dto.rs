use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::accounts::{AccountEntry, BootDecision};

/// One shared shape for an attachment crossing the Tauri IPC boundary in
/// any direction: the result of `pick_attachment`, an input to
/// `send_direct_message`/`send_group_message`, or part of a received
/// `MessageDto`/`ChatEventDto`. `data` only ever becomes `data_base64` at
/// this IPC boundary — Tauri would otherwise JSON-serialize `Vec<u8>` as an
/// array of numbers, several times larger than base64 for no benefit (the
/// same reasoning that drove the P2P wire format to bincode instead of
/// JSON, in `crates/core/src/node.rs`).
#[derive(Serialize, Deserialize, Clone)]
pub struct AttachmentDto {
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub exif_stripped: bool,
    pub data_base64: String,
}

impl From<&storage::StoredAttachment> for AttachmentDto {
    fn from(a: &storage::StoredAttachment) -> Self {
        Self {
            filename: a.filename.clone(),
            mime_type: a.mime_type.clone(),
            size: a.data.len() as u64,
            exif_stripped: a.exif_stripped,
            data_base64: STANDARD.encode(&a.data),
        }
    }
}

impl From<&p2p_core::AttachmentPayload> for AttachmentDto {
    fn from(a: &p2p_core::AttachmentPayload) -> Self {
        Self {
            filename: a.filename.clone(),
            mime_type: a.mime_type.clone(),
            size: a.data.len() as u64,
            exif_stripped: a.exif_stripped,
            data_base64: STANDARD.encode(&a.data),
        }
    }
}

impl AttachmentDto {
    pub fn to_payload(&self) -> Result<p2p_core::AttachmentPayload, String> {
        let data = STANDARD
            .decode(&self.data_base64)
            .map_err(|e| format!("invalid attachment data: {e}"))?;
        Ok(p2p_core::AttachmentPayload {
            filename: self.filename.clone(),
            mime_type: self.mime_type.clone(),
            exif_stripped: self.exif_stripped,
            data,
        })
    }
}

#[derive(Serialize, Clone)]
pub struct ExifFieldDto {
    pub tag: String,
    pub value: String,
}

/// Frontend-facing DTOs. Kept separate from `p2p_core`'s own types because
/// several of those (e.g. carrying a raw libp2p `PeerId`) aren't meant to
/// cross the Tauri IPC boundary as-is.
#[derive(Serialize, Clone)]
pub struct ContactDto {
    pub user_id: String,
    pub display_name: String,
    pub verified: bool,
}

impl From<storage::StoredContact> for ContactDto {
    fn from(c: storage::StoredContact) -> Self {
        Self {
            user_id: c.user_id,
            display_name: c.display_name,
            verified: c.verified,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct MessageDto {
    pub sender_user_id: String,
    pub body: String,
    pub sent_at: i64,
    pub attachment: Option<AttachmentDto>,
}

impl From<storage::StoredMessage> for MessageDto {
    fn from(m: storage::StoredMessage) -> Self {
        Self {
            sender_user_id: m.sender_user_id,
            body: m.body,
            sent_at: m.sent_at,
            attachment: m.attachment.as_ref().map(AttachmentDto::from),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct GroupMemberDto {
    pub user_id: String,
    pub role: String,
}

#[derive(Serialize, Clone)]
pub struct ChannelDto {
    pub channel_id: String,
    pub name: String,
    /// "text" or "voice". Voice channels are created/listed/selected like
    /// any other channel in this phase; the client doesn't yet do anything
    /// with them beyond that — real-time audio is separate, later work.
    pub kind: String,
    pub position: i64,
}

impl From<p2p_core::ChannelInfo> for ChannelDto {
    fn from(c: p2p_core::ChannelInfo) -> Self {
        Self {
            channel_id: c.channel_id,
            name: c.name,
            kind: c.kind.as_str().to_string(),
            position: c.position,
        }
    }
}

impl From<storage::StoredChannel> for ChannelDto {
    fn from(c: storage::StoredChannel) -> Self {
        Self {
            channel_id: c.channel_id,
            name: c.name,
            kind: c.kind,
            position: c.position,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct GroupDto {
    pub group_id: String,
    pub name: String,
    pub roster_version: u64,
    pub members: Vec<GroupMemberDto>,
    pub channels: Vec<ChannelDto>,
}

impl From<p2p_core::GroupInfo> for GroupDto {
    fn from(g: p2p_core::GroupInfo) -> Self {
        Self {
            group_id: g.group_id,
            name: g.name,
            roster_version: g.roster_version,
            members: g
                .members
                .into_iter()
                .map(|m| GroupMemberDto {
                    user_id: m.user_id,
                    role: m.role.as_str().to_string(),
                })
                .collect(),
            channels: g.channels.into_iter().map(ChannelDto::from).collect(),
        }
    }
}

impl From<storage::StoredGroup> for GroupDto {
    fn from(g: storage::StoredGroup) -> Self {
        Self {
            group_id: g.group_id,
            name: g.name,
            roster_version: g.roster_version,
            members: g
                .members
                .into_iter()
                .map(|m| GroupMemberDto {
                    user_id: m.user_id,
                    role: m.role,
                })
                .collect(),
            channels: g.channels.into_iter().map(ChannelDto::from).collect(),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ChatEventDto {
    #[serde(rename = "direct_message")]
    DirectMessage {
        from: String,
        body: String,
        attachment: Option<AttachmentDto>,
    },
    #[serde(rename = "group_message")]
    GroupMessage {
        group_id: String,
        channel_id: String,
        from: String,
        body: String,
        attachment: Option<AttachmentDto>,
    },
    #[serde(rename = "group_key_received")]
    GroupKeyReceived { group_id: String, from: String },
    #[serde(rename = "network_status")]
    NetworkStatus { status: &'static str },
    #[serde(rename = "voice_participants_changed")]
    VoiceParticipantsChanged {
        group_id: String,
        channel_id: String,
        user_ids: Vec<String>,
    },
}

impl TryFrom<p2p_core::ChatEvent> for ChatEventDto {
    type Error = ();

    fn try_from(event: p2p_core::ChatEvent) -> Result<Self, ()> {
        match event {
            p2p_core::ChatEvent::DirectMessage {
                from,
                body,
                attachment,
            } => Ok(ChatEventDto::DirectMessage {
                from,
                body,
                attachment: attachment.as_ref().map(AttachmentDto::from),
            }),
            p2p_core::ChatEvent::GroupMessage {
                group_id,
                channel_id,
                from,
                body,
                attachment,
            } => Ok(ChatEventDto::GroupMessage {
                group_id,
                channel_id,
                from,
                body,
                attachment: attachment.as_ref().map(AttachmentDto::from),
            }),
            p2p_core::ChatEvent::GroupKeyReceived { group_id, from } => {
                Ok(ChatEventDto::GroupKeyReceived { group_id, from })
            }
            p2p_core::ChatEvent::NetworkStatus(status) => Ok(ChatEventDto::NetworkStatus {
                status: match status {
                    p2p_core::events::NetworkStatus::Public => "public",
                    p2p_core::events::NetworkStatus::Private => "private",
                    p2p_core::events::NetworkStatus::Unknown => "unknown",
                },
            }),
            p2p_core::ChatEvent::VoiceParticipantsChanged {
                group_id,
                channel_id,
                user_ids,
            } => Ok(ChatEventDto::VoiceParticipantsChanged {
                group_id,
                channel_id,
                user_ids,
            }),
            // Connection/gossip-subscription events aren't surfaced to the
            // frontend yet — nothing there uses them. Raw `VoicePresence`
            // is always intercepted by `AppService::next_event` itself
            // (translated into `VoiceParticipantsChanged` above, or
            // swallowed if irrelevant) — it should never reach here.
            p2p_core::ChatEvent::Connected(_)
            | p2p_core::ChatEvent::GossipSubscribed { .. }
            | p2p_core::ChatEvent::VoicePresence { .. } => Err(()),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct AccountSummaryDto {
    pub account_id: String,
    pub user_id: String,
    pub display_name: String,
}

impl From<AccountEntry> for AccountSummaryDto {
    fn from(a: AccountEntry) -> Self {
        Self {
            account_id: a.account_id,
            user_id: a.user_id,
            display_name: a.display_name,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct AccountsStateDto {
    pub accounts: Vec<AccountSummaryDto>,
    pub active_account_id: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "action")]
pub enum BootDecisionDto {
    #[serde(rename = "needsFirstAccount")]
    NeedsFirstAccount,
    #[serde(rename = "needsPicker")]
    NeedsPicker { accounts: Vec<AccountSummaryDto> },
    #[serde(rename = "resume")]
    Resume { account: AccountSummaryDto },
    #[serde(rename = "createWithName")]
    CreateWithName { display_name: String },
}

impl From<BootDecision> for BootDecisionDto {
    fn from(d: BootDecision) -> Self {
        match d {
            BootDecision::NeedsFirstAccount => BootDecisionDto::NeedsFirstAccount,
            BootDecision::NeedsPicker(accounts) => BootDecisionDto::NeedsPicker {
                accounts: accounts.into_iter().map(AccountSummaryDto::from).collect(),
            },
            BootDecision::Resume(account) => BootDecisionDto::Resume {
                account: account.into(),
            },
            BootDecision::CreateWithName(display_name) => {
                BootDecisionDto::CreateWithName { display_name }
            }
        }
    }
}
