/** Two short, synthesized tones timed to the boot splash's own visual
 * beats — no audio asset file, generated on the fly with the Web Audio
 * API (same technique as `notificationSound.ts` and `voiceSounds.ts`).
 * Stands in for the video clip's embedded whoosh + chime, which stays
 * muted; see `components/Splash.tsx` for where these get scheduled. */

let audioCtx: AudioContext | null = null;
let gestureFallbackArmed = false;

function getContext(): AudioContext {
  audioCtx ??= new AudioContext();
  return audioCtx;
}

/** Most webviews start a fresh `AudioContext` suspended until a real user
 * gesture has happened, so a tone fired from the splash (well before any
 * click) can silently render nothing. `resume()` is tried right away
 * (some embedded webviews allow it), and if it's still suspended after
 * that, retried on the first pointer/key interaction anywhere on the page
 * so the tones aren't just lost. */
function armGestureFallback(ctx: AudioContext) {
  if (gestureFallbackArmed) return;
  gestureFallbackArmed = true;

  function retry() {
    void ctx.resume();
  }
  document.addEventListener("pointerdown", retry, { once: true });
  document.addEventListener("keydown", retry, { once: true });
}

function ring(ctx: AudioContext, freq: number) {
  const start = ctx.currentTime;
  const end = start + 0.4;

  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = "sine";
  osc.frequency.value = freq;

  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(0.2, start + 0.015);
  gain.gain.exponentialRampToValueAtTime(0.0001, end);

  osc.connect(gain).connect(ctx.destination);
  osc.start(start);
  osc.stop(end + 0.02);
}

type BootTone = "form" | "lock";

const TONE_FREQUENCY: Record<BootTone, number> = {
  form: 880, // A5 — the mark fully forming
  lock: 1318.51, // E6 — the brief light flash just before lock-in
};

/** Play one of the boot splash's two tones — `"form"` then `"lock"`, a
 * rising interval reusing frequencies already established elsewhere in
 * the app's sound palette (`notificationSound.ts`, `voiceSounds.ts`). */
export function playBootTone(tone: BootTone) {
  const ctx = getContext();
  ring(ctx, TONE_FREQUENCY[tone]);

  if (ctx.state === "running") return;
  void ctx.resume();
  armGestureFallback(ctx);
}
