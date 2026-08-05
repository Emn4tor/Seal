import { type RingtoneId, RINGTONES } from "./ringtones";

const SOUND_KEY = "seal-ringtone";
const DEFAULT_RINGTONE: RingtoneId = "classic";

export function getRingtoneId(): RingtoneId {
  const stored = localStorage.getItem(SOUND_KEY);
  return stored && stored in RINGTONES ? (stored as RingtoneId) : DEFAULT_RINGTONE;
}

export function saveRingtoneId(id: RingtoneId) {
  localStorage.setItem(SOUND_KEY, id);
}
