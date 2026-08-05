import { type NotificationSoundId, NOTIFICATION_SOUNDS } from "./notificationSound";

const SOUND_KEY = "seal-notification-sound";
const DEFAULT_SOUND: NotificationSoundId = "classic";

export function getNotificationSoundId(): NotificationSoundId {
  const stored = localStorage.getItem(SOUND_KEY);
  return stored && stored in NOTIFICATION_SOUNDS ? (stored as NotificationSoundId) : DEFAULT_SOUND;
}

export function saveNotificationSoundId(id: NotificationSoundId) {
  localStorage.setItem(SOUND_KEY, id);
}
