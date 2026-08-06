use tokio::sync::RwLock;

use crate::actor::ActorHandle;

/// Holds whichever account is currently loaded, if any, behind a lock
/// instead of a plain `app.manage(ActorHandle)`, so switching accounts can
/// swap the handle in place instead of requiring an app restart. Dropping
/// the old handle (its only `Sender` clone) ends that account's actor task
/// on its own next time it loops around to `rx.recv()`, cleanly tearing down
/// its `AppService` (swarm, DB connection, in-memory KEK) with no extra
/// shutdown code needed.
#[derive(Default)]
pub struct AccountManager {
    current: RwLock<Option<ActorHandle>>,
}

impl AccountManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn current(&self) -> Result<ActorHandle, String> {
        self.current
            .read()
            .await
            .clone()
            .ok_or_else(|| "no account is loaded".to_string())
    }

    pub async fn set_current(&self, handle: ActorHandle) {
        *self.current.write().await = Some(handle);
    }

    pub async fn clear_current(&self) {
        *self.current.write().await = None;
    }
}
