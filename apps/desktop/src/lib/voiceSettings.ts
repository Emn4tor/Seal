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

// Device preferences, by exact name (as returned by `listInputDevices`/
// `listOutputDevices`) — `null`/absent means "use the system default".
// Unlike the mic threshold, these only take effect at the *start* of a
// call: there's no live device-swap mid-call (see `voice.rs`'s
// `spawn_audio_io_thread` doc comment), so there's nothing to push to the
// backend outside of the `joinVoiceChannel` call itself.
const INPUT_DEVICE_KEY = "seal-preferred-input-device";
const OUTPUT_DEVICE_KEY = "seal-preferred-output-device";

export function getPreferredInputDevice(): string | null {
  return localStorage.getItem(INPUT_DEVICE_KEY);
}

export function savePreferredInputDevice(name: string | null) {
  if (name === null) localStorage.removeItem(INPUT_DEVICE_KEY);
  else localStorage.setItem(INPUT_DEVICE_KEY, name);
}

export function getPreferredOutputDevice(): string | null {
  return localStorage.getItem(OUTPUT_DEVICE_KEY);
}

export function savePreferredOutputDevice(name: string | null) {
  if (name === null) localStorage.removeItem(OUTPUT_DEVICE_KEY);
  else localStorage.setItem(OUTPUT_DEVICE_KEY, name);
}
