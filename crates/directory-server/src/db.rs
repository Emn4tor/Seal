use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use wire_proto::{
    ChannelKind, ChannelRecord, GroupMember, GroupRecord, GroupRole, PresenceRecord, UserRecord,
};

use crate::error::AppError;

const SCHEMA: &str = include_str!("schema.sql");

pub fn open(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    migrate_add_share_online_status_column(&conn)?;
    Ok(conn)
}

/// Migrate the database to add the `share_online_status` column bc it missed in the initial schema.
fn migrate_add_share_online_status_column(conn: &Connection) -> anyhow::Result<()> {
    let has_column = conn
        .prepare("SELECT 1 FROM pragma_table_info('presence') WHERE name = 'share_online_status'")?
        .exists([])?;
    if !has_column {
        conn.execute(
            "ALTER TABLE presence ADD COLUMN share_online_status INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    Ok(())
}

// ---- nonce / replay protection ----------------------------------------

const NONCE_WINDOW_SECS: i64 = 600;

/// A generous cap on how many signed requests one identity can make within
/// `NONCE_WINDOW_SECS`, piggybacking on the nonce table every signed write
/// already inserts into rather than needing a new schema, middleware, or
/// per-connection state to rate-limit at all. Not meant to bound legitimate
/// usage, a normal chat session or the 150s presence heartbeat are far
/// under this, only to blunt a compromised or malicious identity flooding
/// writes (presence spam, OTK-upload spam, group-create spam, ...).
pub const MAX_REQUESTS_PER_WINDOW: i64 = 300;

pub fn record_nonce_or_reject(
    conn: &Connection,
    user_id: &str,
    nonce: &str,
    now: i64,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM nonces WHERE seen_at < ?1",
        params![now - NONCE_WINDOW_SECS],
    )?;
    let recent_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nonces WHERE user_id = ?1 AND seen_at >= ?2",
        params![user_id, now - NONCE_WINDOW_SECS],
        |row| row.get(0),
    )?;
    if recent_count >= MAX_REQUESTS_PER_WINDOW {
        return Err(AppError::RateLimited);
    }
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO nonces (user_id, nonce, seen_at) VALUES (?1, ?2, ?3)",
        params![user_id, nonce, now],
    )?;
    if inserted == 0 {
        return Err(AppError::Replay);
    }
    Ok(())
}

// ---- users --------------------------------------------------------------

pub fn insert_user(
    conn: &Connection,
    user_id: &str,
    display_name: &str,
    ed25519_key: &str,
    curve25519_key: &str,
    now: i64,
) -> Result<(), AppError> {
    // Upsert, not insert-only: the client re-asserts registration on every
    // startup (the directory may have been purged since last time), using
    // the same user_id every time since it's a fingerprint of the identity's
    // own key — so a second launch against a directory that still has the
    // first launch's row must succeed, not 409.
    conn.execute(
        "INSERT INTO users (user_id, display_name, ed25519_key, curve25519_key, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(user_id) DO UPDATE SET
            display_name = excluded.display_name,
            ed25519_key = excluded.ed25519_key,
            curve25519_key = excluded.curve25519_key",
        params![user_id, display_name, ed25519_key, curve25519_key, now],
    )?;
    Ok(())
}

pub fn get_user(conn: &Connection, user_id: &str) -> Result<Option<UserRecord>, AppError> {
    conn.query_row(
        "SELECT user_id, display_name, ed25519_key, curve25519_key FROM users WHERE user_id = ?1",
        params![user_id],
        |row| {
            Ok(UserRecord {
                user_id: row.get(0)?,
                display_name: row.get(1)?,
                ed25519_key: row.get(2)?,
                curve25519_key: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

pub fn upload_otks(
    conn: &Connection,
    user_id: &str,
    keys: &[wire_proto::OneTimeKeyEntry],
    fallback: Option<&wire_proto::OneTimeKeyEntry>,
    now: i64,
) -> Result<(), AppError> {
    for k in keys {
        conn.execute(
            "INSERT OR REPLACE INTO one_time_keys (user_id, key_id, public_key, is_fallback, claimed, created_at)
             VALUES (?1, ?2, ?3, 0, 0, ?4)",
            params![user_id, k.key_id, k.public_key, now],
        )?;
    }
    if let Some(fb) = fallback {
        conn.execute(
            "DELETE FROM one_time_keys WHERE user_id = ?1 AND is_fallback = 1",
            params![user_id],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO one_time_keys (user_id, key_id, public_key, is_fallback, claimed, created_at)
             VALUES (?1, ?2, ?3, 1, 0, ?4)",
            params![user_id, fb.key_id, fb.public_key, now],
        )?;
    }
    Ok(())
}

pub fn claim_otk(
    conn: &Connection,
    user_id: &str,
) -> Result<Option<wire_proto::ClaimedOtk>, AppError> {
    let one_time = conn
        .query_row(
            "SELECT key_id, public_key FROM one_time_keys
             WHERE user_id = ?1 AND is_fallback = 0 AND claimed = 0
             ORDER BY created_at ASC LIMIT 1",
            params![user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    if let Some((key_id, public_key)) = one_time {
        // Single-use: delete immediately so it can never be claimed twice.
        conn.execute(
            "DELETE FROM one_time_keys WHERE user_id = ?1 AND key_id = ?2",
            params![user_id, key_id],
        )?;
        return Ok(Some(wire_proto::ClaimedOtk {
            key_id,
            public_key,
            is_fallback: false,
        }));
    }

    // No one-time keys left: fall back to the reusable fallback key, if any.
    let fallback = conn
        .query_row(
            "SELECT key_id, public_key FROM one_time_keys WHERE user_id = ?1 AND is_fallback = 1",
            params![user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    Ok(fallback.map(|(key_id, public_key)| wire_proto::ClaimedOtk {
        key_id,
        public_key,
        is_fallback: true,
    }))
}

// ---- presence -------------------------------------------------------------

/// Deletes presence rows whose TTL has already lapsed. `get_presence`
/// already filters these out of reads (`WHERE expires_at > now`), so this
/// isn't needed for correctness — it's for minimizing what a seized/leaked
/// copy of the database actually contains. Without it, an expired row (a
/// peer_id <-> IP/relay-circuit mapping) just sits there forever once a user
/// stops heartbeating, readable in a disk dump long after it stopped being
/// true. Called opportunistically on every presence write, same pattern as
/// the nonce sweep above, so it stays current without needing its own
/// scheduled job.
pub fn sweep_expired_presence(conn: &Connection, now: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM presence WHERE expires_at < ?1", params![now])?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_presence(
    conn: &Connection,
    user_id: &str,
    peer_id: &str,
    multiaddrs: &[String],
    relay_addrs: &[String],
    share_online_status: bool,
    expires_at: i64,
    now: i64,
) -> Result<(), AppError> {
    sweep_expired_presence(conn, now)?;
    let multiaddrs_json = serde_json::to_string(multiaddrs).map_err(anyhow::Error::new)?;
    let relay_addrs_json = serde_json::to_string(relay_addrs).map_err(anyhow::Error::new)?;
    conn.execute(
        "INSERT INTO presence (user_id, peer_id, multiaddrs, relay_addrs, share_online_status, expires_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id) DO UPDATE SET
            peer_id = excluded.peer_id,
            multiaddrs = excluded.multiaddrs,
            relay_addrs = excluded.relay_addrs,
            share_online_status = excluded.share_online_status,
            expires_at = excluded.expires_at,
            updated_at = excluded.updated_at",
        params![
            user_id,
            peer_id,
            multiaddrs_json,
            relay_addrs_json,
            share_online_status,
            expires_at,
            now
        ],
    )?;
    Ok(())
}

pub fn get_presence(
    conn: &Connection,
    user_id: &str,
    now: i64,
) -> Result<Option<PresenceRecord>, AppError> {
    let row = conn
        .query_row(
            "SELECT peer_id, multiaddrs, relay_addrs, share_online_status, expires_at FROM presence
             WHERE user_id = ?1 AND expires_at > ?2",
            params![user_id, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;

    let Some((peer_id, multiaddrs_json, relay_addrs_json, share_online_status, expires_at)) = row
    else {
        return Ok(None);
    };
    let multiaddrs: Vec<String> =
        serde_json::from_str(&multiaddrs_json).map_err(anyhow::Error::new)?;
    let relay_addrs: Vec<String> =
        serde_json::from_str(&relay_addrs_json).map_err(anyhow::Error::new)?;
    Ok(Some(PresenceRecord {
        user_id: user_id.to_string(),
        peer_id,
        multiaddrs,
        relay_addrs,
        expires_at,
        share_online_status,
    }))
}

// ---- groups -----------------------------------------------------------------

pub fn create_group(
    conn: &Connection,
    group_id: &str,
    name: &str,
    creator_id: &str,
    now: i64,
) -> Result<GroupRecord, AppError> {
    conn.execute(
        "INSERT INTO groups (group_id, name, roster_version, created_at) VALUES (?1, ?2, 1, ?3)",
        params![group_id, name, now],
    )?;
    conn.execute(
        "INSERT INTO group_members (group_id, user_id, role, added_at) VALUES (?1, ?2, 'owner', ?3)",
        params![group_id, creator_id, now],
    )?;
    // A group is never left channel-less — every group gets a default text
    // channel the moment it's created, mirroring the "#general" every
    // Discord-style server starts with.
    let default_channel_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO channels (channel_id, group_id, name, kind, position, created_at)
         VALUES (?1, ?2, 'general', 'text', 0, ?3)",
        params![default_channel_id, group_id, now],
    )?;
    Ok(GroupRecord {
        group_id: group_id.to_string(),
        name: name.to_string(),
        roster_version: 1,
        members: vec![GroupMember {
            user_id: creator_id.to_string(),
            role: GroupRole::Owner,
        }],
        channels: vec![ChannelRecord {
            channel_id: default_channel_id,
            group_id: group_id.to_string(),
            name: "general".to_string(),
            kind: ChannelKind::Text,
            position: 0,
        }],
    })
}

pub fn get_group(conn: &Connection, group_id: &str) -> Result<Option<GroupRecord>, AppError> {
    let group = conn
        .query_row(
            "SELECT name, roster_version FROM groups WHERE group_id = ?1",
            params![group_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((name, roster_version)) = group else {
        return Ok(None);
    };
    let members = list_members(conn, group_id)?;
    let channels = list_channels(conn, group_id)?;
    Ok(Some(GroupRecord {
        group_id: group_id.to_string(),
        name,
        roster_version: roster_version as u64,
        members,
        channels,
    }))
}

/// Owner-only in practice — the permission check lives in the route handler
/// (same pattern as roster updates), not here; this just inserts.
pub fn create_channel(
    conn: &Connection,
    group_id: &str,
    channel_id: &str,
    name: &str,
    kind: ChannelKind,
    now: i64,
) -> Result<ChannelRecord, AppError> {
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE group_id = ?1",
        params![group_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO channels (channel_id, group_id, name, kind, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![channel_id, group_id, name, kind.as_str(), position, now],
    )?;
    Ok(ChannelRecord {
        channel_id: channel_id.to_string(),
        group_id: group_id.to_string(),
        name: name.to_string(),
        kind,
        position,
    })
}

fn list_channels(conn: &Connection, group_id: &str) -> Result<Vec<ChannelRecord>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT channel_id, name, kind, position FROM channels
         WHERE group_id = ?1 ORDER BY position ASC",
    )?;
    let rows = stmt
        .query_map(params![group_id], |row| {
            let channel_id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let position: i64 = row.get(3)?;
            Ok((channel_id, name, kind, position))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(channel_id, name, kind, position)| ChannelRecord {
            channel_id,
            group_id: group_id.to_string(),
            name,
            kind: if kind == "voice" {
                ChannelKind::Voice
            } else {
                ChannelKind::Text
            },
            position,
        })
        .collect())
}

fn list_members(conn: &Connection, group_id: &str) -> Result<Vec<GroupMember>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT user_id, role FROM group_members WHERE group_id = ?1 ORDER BY added_at ASC",
    )?;
    let rows = stmt
        .query_map(params![group_id], |row| {
            let user_id: String = row.get(0)?;
            let role: String = row.get(1)?;
            Ok((user_id, role))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(user_id, role)| GroupMember {
            user_id,
            role: if role == "owner" {
                GroupRole::Owner
            } else {
                GroupRole::Member
            },
        })
        .collect())
}

/// The group_ids a user belongs to, with no other detail — just enough
/// for a client to notice "I'm apparently in a group I don't have
/// locally" and go fetch/request the rest. See the doc comment on the
/// `/v1/users/{user_id}/groups` route for why this exists: without it, a
/// client that missed the one-shot P2P message that normally delivers
/// membership (offline, a dropped dial, anything) had no way to ever find
/// out, since group_ids aren't otherwise discoverable.
pub fn groups_for_user(conn: &Connection, user_id: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT group_id FROM group_members WHERE user_id = ?1")?;
    let rows = stmt
        .query_map(params![user_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn member_role(
    conn: &Connection,
    group_id: &str,
    user_id: &str,
) -> Result<Option<GroupRole>, AppError> {
    let role: Option<String> = conn
        .query_row(
            "SELECT role FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(role.map(|r| {
        if r == "owner" {
            GroupRole::Owner
        } else {
            GroupRole::Member
        }
    }))
}

pub fn roster_version(conn: &Connection, group_id: &str) -> Result<Option<u64>, AppError> {
    let v: Option<i64> = conn
        .query_row(
            "SELECT roster_version FROM groups WHERE group_id = ?1",
            params![group_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(v.map(|v| v as u64))
}

pub fn apply_roster_update(
    conn: &Connection,
    group_id: &str,
    add: &[String],
    remove: &[String],
    new_version: u64,
    now: i64,
) -> Result<GroupRecord, AppError> {
    for user_id in add {
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, user_id, role, added_at)
             VALUES (?1, ?2, 'member', ?3)",
            params![group_id, user_id, now],
        )?;
    }
    for user_id in remove {
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
    }
    conn.execute(
        "UPDATE groups SET roster_version = ?1 WHERE group_id = ?2",
        params![new_version as i64, group_id],
    )?;
    get_group(conn, group_id)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("group vanished during roster update")))
}
