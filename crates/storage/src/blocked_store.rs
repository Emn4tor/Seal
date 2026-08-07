use rusqlite::params;

use crate::error::StorageError;
use crate::store::LocalStore;

impl LocalStore {
    pub fn block_user(&self, user_id: &str, blocked_at: i64) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO blocked_contacts (user_id, blocked_at) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO NOTHING",
            params![user_id, blocked_at],
        )?;
        Ok(())
    }

    pub fn unblock_user(&self, user_id: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM blocked_contacts WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }

    pub fn is_blocked(&self, user_id: &str) -> Result<bool, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM blocked_contacts WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn list_blocked(&self) -> Result<Vec<String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id FROM blocked_contacts ORDER BY blocked_at DESC")?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
