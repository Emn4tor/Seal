/** Voice preferences persisted locally (per-device, not synced) and
 * re-applied to the backend whenever a call starts or the user changes
 * them live. */

const THRESHOLD_KEY = "seal-mic-threshold-db";
export const DEFAULT_MIC_THRESHOLD_DB = -50;
export const MIC_THRESHOLD_MIN_DB = -60;
export const MIC_THRESHOLD_MAX_DB = -10;

export function getMicThresholdDb(): number {
  const raw = localStorage.getItem(THRESHOLD_KEY);
  const value = raw !== null ? Number(raw) : DEFAULT_MIC_THRESHOLD_DB;
  return Number.isFinite(value) ? value : DEFAULT_MIC_THRESHOLD_DB;
}

export function saveMicThresholdDb(db: number) {
  localStorage.setItem(THRESHOLD_KEY, String(db));
}
