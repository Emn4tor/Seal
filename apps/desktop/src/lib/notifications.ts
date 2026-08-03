import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { playMessageSound } from "./notificationSound";

const NOTIFICATIONS_KEY = "seal-notifications-enabled";

export function getNotificationsEnabled(): boolean {
  const stored = localStorage.getItem(NOTIFICATIONS_KEY);
  return stored === null ? true : stored === "true";
}

export function saveNotificationsEnabled(enabled: boolean) {
  localStorage.setItem(NOTIFICATIONS_KEY, String(enabled));
}

let permissionRequested = false;

/** Requests OS notification permission at most once per launch — safe to
 * call speculatively (e.g. at startup) since it no-ops if already
 * granted/denied. */
export async function ensureNotificationPermission(): Promise<boolean> {
  if (await isPermissionGranted().catch(() => false)) return true;
  if (permissionRequested) return false;
  permissionRequested = true;
  const result = await requestPermission().catch(() => "denied" as const);
  return result === "granted";
}

/** Shows a native OS notification for an incoming message and plays Seal's
 * own chime instead of the OS's default notification sound (never set on
 * the notification itself), so it sounds the same and stays recognizable
 * across macOS/Windows/Linux. No-ops entirely if the user has turned
 * notifications off in Settings. */
export async function notifyNewMessage(title: string, body: string) {
  if (!getNotificationsEnabled()) return;
  playMessageSound();
  if (await ensureNotificationPermission()) {
    sendNotification({ title, body });
  }
}
