use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use wire_proto::RelayInfoResponse;

use crate::db;
use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    conn: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    /// Set once at startup from the relay's own (synchronously known)
    /// keypair — `None` when no `DIRECTORY_RELAY_EXTERNAL_MULTIADDR` is
    /// configured, in which case `/v1/relay-info` reports itself absent
    /// rather than advertising an address nobody outside this host can
    /// actually reach.
    pub relay_info: Option<RelayInfoResponse>,
}

impl AppState {
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        let conn = db::open(&db_path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
            relay_info: None,
        })
    }

    pub fn with_relay_info(mut self, relay_info: Option<RelayInfoResponse>) -> Self {
        self.relay_info = relay_info;
        self
    }

    /// Runs a blocking rusqlite closure off the async runtime's worker threads.
    ///
    /// Recovers from a poisoned lock rather than panicking on it: a single
    /// request's closure panicking mid-call would otherwise poison this
    /// one shared `Mutex` forever, taking down every future request from
    /// every user with the same panic rather than just failing that one
    /// request. Nothing here holds a Rust-level invariant across separate
    /// calls, only whatever SQLite itself already guarantees per
    /// statement, so recovering the connection and letting the next
    /// request try again is the right tradeoff over a single point of
    /// failure for the whole server.
    pub async fn with_conn<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, AppError> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            f(&guard)
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("blocking task join error: {e}")))?
    }

    /// Instantly and irrecoverably wipes all directory/presence/group data.
    /// Deletes the SQLite file (+ WAL/SHM) and recreates an empty schema.
    pub async fn purge(&self) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        let path = self.db_path.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let placeholder = Connection::open_in_memory()?;
            let old = std::mem::replace(&mut *guard, placeholder);
            old.close().map_err(|(_, e)| e)?;
            for suffix in ["", "-wal", "-shm"] {
                let mut p = path.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(p);
            }
            *guard = db::open(&path)?;
            Ok(())
        })
        .await??;
        Ok(())
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic inside one request's `with_conn` closure used to poison the
    /// shared connection mutex for good, taking down every request after
    /// it too. This proves a second, well-behaved call still succeeds.
    #[tokio::test]
    async fn a_panicking_request_does_not_take_down_the_ones_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(dir.path().join("directory.sqlite3")).unwrap();

        let panicked = state
            .with_conn(|_conn| -> Result<(), AppError> {
                panic!("simulating a bug in one request's handler");
            })
            .await;
        assert!(
            panicked.is_err(),
            "the panicking request itself still fails"
        );

        let recovered = state
            .with_conn(|conn| {
                conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map_err(AppError::from)
            })
            .await;
        assert_eq!(
            recovered.unwrap(),
            1,
            "a later request must still be able to use the connection"
        );
    }
}
