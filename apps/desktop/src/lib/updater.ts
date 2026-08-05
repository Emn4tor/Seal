import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export async function checkForUpdates(): Promise<boolean> {
  const update = await check();
  return update ? true : false;
}

export async function update() {
  const update = await check();
  if (!update) return;
  await update.downloadAndInstall();
  await relaunch();
}