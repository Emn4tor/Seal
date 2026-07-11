use rusqlite::params;

use crate::error::StorageError;
use crate::store::LocalStore;

#[derive(Debug, Clone)]
pub struct StoredGroupMember {
    pub user_id: String,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct StoredChannel {
    pub channel_id: String,
    pub name: String,
    pub kind: String,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct StoredGroup {
    pub group_id: String,
    pub name: String,
    pub roster_version: u64,
    pub members: Vec<StoredGroupMember>,
    pub channels: Vec<StoredChannel>,
}

impl LocalStore {
    pub fn upsert_group(
        &self,
        group_id: &str,
        name: &str,
        roster_version: u64,
        members: &[(String, String)],
        channels: &[(String, String, String, i64)],
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO groups (group_id, name, roster_version) VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id) DO UPDATE SET
                name = excluded.name,
                roster_version = excluded.roster_version",
            params![group_id, name, roster_version as i64],
        )?;
        self.conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1",
            params![group_id],
        )?;
        for (user_id, role) in members {
            self.conn.execute(
                "INSERT INTO group_members (group_id, user_id, role) VALUES (?1, ?2, ?3)",
                params![group_id, user_id, role],
            )?;
        }
        self.conn.execute(
            "DELETE FROM channels WHERE group_id = ?1",
            params![group_id],
        )?;
        for (channel_id, channel_name, kind, position) in channels {
            self.conn.execute(
                "INSERT INTO channels (channel_id, group_id, name, kind, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![channel_id, group_id, channel_name, kind, position],
            )?;
        }
        Ok(())
    }

    pub fn list_groups(&self) -> Result<Vec<StoredGroup>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT group_id, name, roster_version FROM groups ORDER BY name ASC")?;
        let groups = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        groups
            .into_iter()
            .map(|(group_id, name, roster_version)| {
                let members = self.group_members(&group_id)?;
                let channels = self.group_channels(&group_id)?;
                Ok(StoredGroup {
                    group_id,
                    name,
                    roster_version: roster_version as u64,
                    members,
                    channels,
                })
            })
            .collect()
    }

    fn group_members(&self, group_id: &str) -> Result<Vec<StoredGroupMember>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id, role FROM group_members WHERE group_id = ?1")?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok(StoredGroupMember {
                    user_id: row.get(0)?,
                    role: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn group_channels(&self, group_id: &str) -> Result<Vec<StoredChannel>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT channel_id, name, kind, position FROM channels
             WHERE group_id = ?1 ORDER BY position ASC",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok(StoredChannel {
                    channel_id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    position: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
