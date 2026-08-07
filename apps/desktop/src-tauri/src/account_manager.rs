use tokio::sync::RwLock;

use crate::actor::ActorHandle;

/// Holds whichever account is currently loaded, if any, behind a lock
/// instead of a plain `app.manage(ActorHandle)`, so switching accounts can
/// swap the handle in place instead of requiring an app restart.
///
/// Keeping the spawned task's `JoinHandle` alongside its `ActorHandle` (not
/// just the handle alone) is what makes `set_current`/`clear_current`
/// actually wait for the outgoing account's `run()` loop to exit before
/// returning, instead of only dropping this struct's one `Sender` clone and
/// trusting the task to notice on its own time. Without that wait, a
/// caller that immediately deletes the account's files afterward (removing
/// an account, panic-purging, switching accounts) could race a still-live
/// `AppService` still holding that same database open, and a caller that
/// immediately starts a new account's task could receive an event the old
/// one emits on its way out.
#[derive(Default)]
pub struct AccountManager {
    current: RwLock<Option<(ActorHandle, tauri::async_runtime::JoinHandle<()>)>>,
}

impl AccountManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn current(&self) -> Result<ActorHandle, String> {
        self.current
            .read()
            .await
            .as_ref()
            .map(|(handle, _)| handle.clone())
            .ok_or_else(|| "no account is loaded".to_string())
    }

    /// Replaces whichever account is currently loaded (if any) with a
    /// freshly spawned one, waiting for the old one to fully shut down
    /// first. See this struct's doc comment for why that wait matters.
    pub async fn set_current(
        &self,
        handle: ActorHandle,
        join: tauri::async_runtime::JoinHandle<()>,
    ) {
        let previous = self.current.write().await.replace((handle, join));
        Self::await_shutdown(previous).await;
    }

    pub async fn clear_current(&self) {
        let previous = self.current.write().await.take();
        Self::await_shutdown(previous).await;
    }

    async fn await_shutdown(previous: Option<(ActorHandle, tauri::async_runtime::JoinHandle<()>)>) {
        let Some((handle, join)) = previous else {
            return;
        };
        // The lock is already released by this point (the caller only held
        // it long enough to swap the value out), so this wait can't stall
        // an unrelated `current()` read from another command in the
        // meantime, it'll just correctly see "no account loaded" (or the
        // new one) while this finishes in the background of the await.
        drop(handle);
        let _ = join.await;
    }
}
