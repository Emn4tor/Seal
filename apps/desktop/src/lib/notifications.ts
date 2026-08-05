import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { playMessageSound } from "./notificationSound";
import { getDndEnabled } from "./dndSettings";
import { getNotificationSoundId } from "./notificationSoundSettings";

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

/** Plays the currently-selected message chime, unless Do Not Disturb is on.
 * Split out from `notifyNewMessage` so a caller can play the sound without
 * necessarily also showing a popup (see `App.tsx`'s `notify` helper: a
 * message in the conversation you're already looking at still gets a
 * sound, just not an interrupting popup). Still respects the notifications
 * on/off switch — DND is an *additional* silencer, not a replacement for
 * it. */
export function playNotificationSound() {
  if (!getNotificationsEnabled() || getDndEnabled()) return;
  playMessageSound(getNotificationSoundId());
}

/** Shows a native OS notification for an incoming message. Does *not* play
 * a sound itself — call `playNotificationSound()` separately (the two are
 * intentionally decoupled; see its doc comment). No-ops entirely if the
 * user has turned notifications off in Settings. */
export async function showNotificationPopup(title: string, body: string) {
  if (!getNotificationsEnabled()) return;
  if (await ensureNotificationPermission()) {
    sendNotification({ title, body });
  }
}

/** Sound + popup together — the common case for a message in a conversation
 * that isn't currently open. */
export async function notifyNewMessage(title: string, body: string) {
  playNotificationSound();
  await showNotificationPopup(title, body);
}
