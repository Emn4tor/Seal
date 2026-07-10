use serde::{Deserialize, Serialize};

use crate::signing::join;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub group_id: String,
    pub name: String,
    pub creator_id: String,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl CreateGroupRequest {
    pub const DOMAIN: &'static str = "create-group/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.group_id,
                &self.name,
                &self.creator_id,
                &ts,
                &self.nonce,
            ],
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupRole {
    Owner,
    Member,
}

impl GroupRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupRole::Owner => "owner",
            GroupRole::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub user_id: String,
    pub role: GroupRole,
}

/// Text channels carry regular chat messages; voice channels are a
/// placeholder in this phase (created/listed/selected like any other
/// channel, but the client doesn't yet do anything with them beyond that —
/// real-time audio is a separate, later phase of work).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
}

impl ChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelKind::Text => "text",
            ChannelKind::Voice => "voice",
        }
    }
}

/// Channel name/kind/position is structural metadata, not message content —
/// exactly like the group name and member list already aren't — so it lives
/// on the directory server the same way, not synced peer-to-peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub channel_id: String,
    pub group_id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRecord {
    pub group_id: String,
    pub name: String,
    pub roster_version: u64,
    pub members: Vec<GroupMember>,
    pub channels: Vec<ChannelRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterUpdateRequest {
    pub group_id: String,
    pub actor_id: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub expected_version: u64,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl RosterUpdateRequest {
    pub const DOMAIN: &'static str = "roster-update/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let add = self.add.join(",");
        let remove = self.remove.join(",");
        let ver = self.expected_version.to_string();
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.group_id,
                &self.actor_id,
                &add,
                &remove,
                &ver,
                &ts,
                &self.nonce,
            ],
        )
    }
}

/// Owner-only (enforced server-side, same `member_role` check already used
/// for roster updates) — creates one new channel in an existing group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub group_id: String,
    pub channel_id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub actor_id: String,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: String,
}

impl CreateChannelRequest {
    pub const DOMAIN: &'static str = "create-channel/v1";

    pub fn signing_bytes(&self) -> Vec<u8> {
        let ts = self.timestamp.to_string();
        join(
            Self::DOMAIN,
            &[
                &self.group_id,
                &self.channel_id,
                &self.name,
                self.kind.as_str(),
                &self.actor_id,
                &ts,
                &self.nonce,
            ],
        )
    }
}
