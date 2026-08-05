/** Per-group muting: silences both the sound and the popup/toast for one
 * group's messages, either for a fixed duration or indefinitely — distinct
 * from Do Not Disturb (`dndSettings.ts`), which is global and sound-only.
 * Keyed by `group_id` (a mute applies to every channel in the group, not
 * one channel at a time — group chat, like a Discord server, is the level
 * people actually think of as "noisy" or not). */

export interface MuteEntry {
  /** `null` means muted forever; otherwise a `Date.now()`-comparable
   * timestamp (ms) after which the mute no longer applies. */
  until: number | null;
}

const MUTED_KEY = "seal-muted-groups";

function readAll(): Record<string, MuteEntry> {
  try {
    const raw = localStorage.getItem(MUTED_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function writeAll(entries: Record<string, MuteEntry>) {
  localStorage.setItem(MUTED_KEY, JSON.stringify(entries));
}

/** `null` durationMs mutes forever; otherwise mutes for that many ms from now. */
export function muteGroup(groupId: string, durationMs: number | null) {
  const entries = readAll();
  entries[groupId] = { until: durationMs === null ? null : Date.now() + durationMs };
  writeAll(entries);
}

export function unmuteGroup(groupId: string) {
  const entries = readAll();
  delete entries[groupId];
  writeAll(entries);
}

/** Whether a group is currently muted — an expired timed mute is treated
 * as not-muted (and opportunistically cleaned up) rather than requiring a
 * separate sweep. */
export function isGroupMuted(groupId: string): boolean {
  const entries = readAll();
  const entry = entries[groupId];
  if (!entry) return false;
  if (entry.until !== null && entry.until <= Date.now()) {
    delete entries[groupId];
    writeAll(entries);
    return false;
  }
  return true;
}

/** `null` if not muted at all. */
export function getGroupMuteEntry(groupId: string): MuteEntry | null {
  return isGroupMuted(groupId) ? readAll()[groupId] : null;
}
