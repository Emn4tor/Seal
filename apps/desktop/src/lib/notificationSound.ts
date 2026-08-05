/** Short, synthesized chimes for incoming messages — no audio asset files,
 * generated on the fly with the Web Audio API (same technique as
 * `voiceSounds.ts`). Several options rather than one fixed sound, picked in
 * Settings and persisted via `notificationSoundSettings.ts`. */

export type NotificationSoundId = "classic" | "deep" | "resonant";

interface ToneNote {
  freq: number;
  type: OscillatorType;
}

interface ToneSpec {
  notes: ToneNote[];
  noteMs: number;
  /** How much each note overlaps the previous one, as a fraction of
   * `noteMs` — 1 means back-to-back, less than 1 lets them overlap
   * slightly for a smoother run. */
  spacing: number;
  peakGain: number;
}

export const NOTIFICATION_SOUNDS: Record<NotificationSoundId, { label: string; description: string }> = {
  classic: { label: "Classic", description: "A bright, quick three-note pling." },
  deep: { label: "Deep", description: "A lower, mellower two-note tone." },
  resonant: {
    label: "Resonant",
    description: "In the same register as the voice-channel chime, but a distinct three-note run so the two stay easy to tell apart by ear.",
  },
};

const SPECS: Record<NotificationSoundId, ToneSpec> = {
  // The original: a bright ascending triad (A5 -> C#6 -> E6), quick and bell-like.
  classic: {
    notes: [
      { freq: 880, type: "triangle" },
      { freq: 1108.73, type: "triangle" },
      { freq: 1318.51, type: "triangle" },
    ],
    noteMs: 70,
    spacing: 0.85,
    peakGain: 0.18,
  },
  // A full two octaves lower and sine-toned throughout: warmer, less
  // attention-grabbing, for anyone who finds "classic" too sharp.
  deep: {
    notes: [
      { freq: 220, type: "sine" },
      { freq: 174.61, type: "sine" },
    ],
    noteMs: 110,
    spacing: 0.9,
    peakGain: 0.22,
  },
  // Same base register and rising shape as `voiceSounds.ts`'s join chime
  // ([440, 660], sine) so the two feel like they belong to the same sound
  // family, but three notes instead of two and triangle instead of sine —
  // enough difference to tell them apart at a glance-of-the-ear, not just
  // on close listening.
  resonant: {
    notes: [
      { freq: 440, type: "triangle" },
      { freq: 554.37, type: "triangle" },
      { freq: 660, type: "triangle" },
    ],
    noteMs: 80,
    spacing: 0.8,
    peakGain: 0.2,
  },
};

let audioCtx: AudioContext | null = null;

function getContext(): AudioContext {
  audioCtx ??= new AudioContext();
  return audioCtx;
}

export function playMessageSound(soundId: NotificationSoundId = "classic") {
  const spec = SPECS[soundId] ?? SPECS.classic;
  const ctx = getContext();
  const start0 = ctx.currentTime;

  spec.notes.forEach(({ freq, type }, i) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = type;
    osc.frequency.value = freq;

    const start = start0 + i * (spec.noteMs / 1000) * spec.spacing;
    const end = start + spec.noteMs / 1000 + 0.12;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(spec.peakGain, start + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, end);

    osc.connect(gain).connect(ctx.destination);
    osc.start(start);
    osc.stop(end + 0.02);
  });
}
