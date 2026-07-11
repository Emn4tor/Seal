use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db;
use crate::error::StorageError;

pub struct LocalStore {
    pub(crate) conn: Connection,
    pub(crate) kek: [u8; 32],
    pub db_path: PathBuf,
}

impl LocalStore {
    pub fn open(db_path: &Path, kek: [u8; 32]) -> Result<Self, StorageError> {
        let conn = db::open(db_path)?;
        Ok(Self {
            conn,
            kek,
            db_path: db_path.to_path_buf(),
        })
    }
}
