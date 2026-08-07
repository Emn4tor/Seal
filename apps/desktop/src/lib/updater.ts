import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export async function checkForUpdates(): Promise<boolean> {
  const update = await check();
  return update ? true : false;
}

export async function update() {
  const update = await check();
  if (!update) {
    // Can legitimately happen if the update that triggered the "Update
    // available" modal is no longer there by the time the user clicks
    // through (installed some other way, or the check was transient).
    // Throwing rather than silently returning lets the modal's own error
    // display explain that instead of just closing with nothing having
    // happened and no indication why.
    throw new Error("No update is available anymore, nothing to install.");
  }
  await update.downloadAndInstall();
  await relaunch();
}