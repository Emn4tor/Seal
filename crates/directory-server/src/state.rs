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
    pub async fn with_conn<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, AppError> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().expect("db mutex poisoned");
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
            let mut guard = conn.lock().expect("db mutex poisoned");
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
