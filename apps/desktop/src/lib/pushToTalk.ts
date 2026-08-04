import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { api } from "./tauri";

/** System-wide push-to-talk: works no matter which app has focus, since
 * it's a real OS-level global shortcut (`tauri-plugin-global-shortcut`),
 * not a browser/window keyboard listener. */

const ENABLED_KEY = "seal-ptt-enabled";
const SHORTCUT_KEY = "seal-ptt-shortcut";

export function getPttEnabled(): boolean {
  return localStorage.getItem(ENABLED_KEY) === "true";
}

export function getPttShortcut(): string | null {
  return localStorage.getItem(SHORTCUT_KEY);
}

export function savePttSettings(enabled: boolean, shortcut: string | null) {
  localStorage.setItem(ENABLED_KEY, String(enabled));
  if (shortcut) localStorage.setItem(SHORTCUT_KEY, shortcut);
  else localStorage.removeItem(SHORTCUT_KEY);
}

/** The physical keys that are themselves modifiers — never valid as the
 * "main" key of a combo, so a recorder should keep waiting past these. */
const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
]);

/** Builds a `tauri-plugin-global-shortcut` accelerator string (e.g.
 * "Alt+KeyV") straight from a physical, layout-independent key event.
 * Returns `null` while only modifiers are held — call again on the next
 * keydown once a real key joins them. */
export function acceleratorFromKeyEvent(e: { code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  parts.push(e.code);
  return parts.join("+");
}

let registeredShortcut: string | null = null;

/** Registers (or unregisters) the global shortcut to match the desired
 * state — safe to call repeatedly (e.g. on every settings change and once
 * at app startup); it only actually re-registers when something changed.
 * While push-to-talk is active, the mic starts muted (the "talk" key is
 * what unmutes it, not the other way around).
 */
export async function applyPushToTalk(enabled: boolean, shortcut: string | null) {
  const desired = enabled ? shortcut : null;
  if (registeredShortcut === desired) return;

  if (registeredShortcut) {
    await unregister(registeredShortcut).catch(() => {});
    registeredShortcut = null;
    // Losing PTT's shortcut — disabling it, or clearing the key while it's
    // still nominally on — used to leave the mic exactly as it was the
    // instant you stopped holding the key: permanently muted if you
    // weren't actively talking at that moment, with nothing left to ever
    // unmute it again. `!desired` (not just `!enabled`) catches both cases
    // and hands control back to a normal always-on mic either way. Only
    // fires here, not below, since re-registering a *different* shortcut
    // while staying enabled shouldn't touch mute state at all.
    if (!desired) api.setMicMuted(false).catch(() => {});
  }
  if (!desired) return;

  await register(desired, (event) => {
    if (event.state === "Pressed") {
      api.setMicMuted(false).catch(() => {});
    } else if (event.state === "Released") {
      api.setMicMuted(true).catch(() => {});
    }
  });
  registeredShortcut = desired;
  // No-ops harmlessly if there's no account/call active yet (e.g. this
  // runs once at app startup before login) — nothing to mute yet.
  api.setMicMuted(true).catch(() => {});
}
