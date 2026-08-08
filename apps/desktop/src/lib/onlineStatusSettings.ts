/** Whether this account shares its online status with contacts. Persisted
 * locally (per-device) and re-applied to the backend on startup and
 * whenever the user changes it live, same pattern as `voiceSettings.ts`'s
 * mic threshold. Enabled by default: only an explicit opt-out turns it
 * off. Doesn't affect whether contacts can still reach this account
 * (`AppService`'s presence heartbeat keeps running either way) — only
 * whether contacts' clients are allowed to show a status dot for it. */

const SHARE_ONLINE_STATUS_KEY = "seal-share-online-status";

export function getShareOnlineStatus(): boolean {
  const raw = localStorage.getItem(SHARE_ONLINE_STATUS_KEY);
  return raw === null ? true : raw === "true";
}

export function saveShareOnlineStatus(enabled: boolean) {
  localStorage.setItem(SHARE_ONLINE_STATUS_KEY, String(enabled));
}
