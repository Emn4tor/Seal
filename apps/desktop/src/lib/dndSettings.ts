/** Do Not Disturb: a single global switch that silences every message sound
 * (the notification chime, regardless of which one is selected) without
 * touching anything visual — the native OS notification and in-app toast
 * still show normally. A blanket mute of *sound only*, distinct from
 * per-group muting (`mutedConversations.ts`), which silences everything
 * (sound and popup) but only for one group. */

const DND_KEY = "seal-dnd-enabled";

export function getDndEnabled(): boolean {
  return localStorage.getItem(DND_KEY) === "true";
}

export function saveDndEnabled(enabled: boolean) {
  localStorage.setItem(DND_KEY, String(enabled));
}
