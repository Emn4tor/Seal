import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

/** Whether Seal launches itself when you log into this device
 * (`tauri-plugin-autostart` — a real OS-level launch agent/registry
 * entry/desktop-file autostart, not something the app has to re-arm on
 * every run). Decided once at onboarding, changeable later in Settings. */
const AUTOSTART_KEY = "seal-autostart-enabled";

export function getAutostartEnabled(): boolean {
  const stored = localStorage.getItem(AUTOSTART_KEY);
  // Default on: a message app you don't notice missed messages from.
  return stored === null ? true : stored === "true";
}

export function saveAutostartEnabled(enabled: boolean) {
  localStorage.setItem(AUTOSTART_KEY, String(enabled));
}

/** Reconciles the OS-level autostart registration with the saved
 * preference — safe to call on every launch; it only touches the OS state
 * when it's out of sync. */
export async function syncAutostart(desired: boolean) {
  const current = await isEnabled().catch(() => desired);
  if (current === desired) return;
  if (desired) await enable().catch(() => {});
  else await disable().catch(() => {});
}
