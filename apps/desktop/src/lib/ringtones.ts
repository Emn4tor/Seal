/** Synthesized ringtones for incoming 1:1 calls — no audio asset files,
 * generated on the fly with the Web Audio API (same technique as
 * `notificationSound.ts`/`voiceSounds.ts`). Unlike a notification sound
 * (one short one-shot chime), a ringtone needs to *loop* for as long as a
 * call keeps ringing, so this exports a start/stop controller instead of a
 * fire-and-forget play function. Five options, picked in Settings and
 * persisted via `ringtoneSettings.ts`. */

export type RingtoneId = "classic" | "chimes" | "digital" | "marimba" | "pulse";

interface Chord {
  /** One or more frequencies played together — a single note for a length-1
   * array, or a dual/triple tone burst for a phone-bell-like chord. */
  freqs: number[];
  type: OscillatorType;
}

interface RingtoneSpec {
  /** One pass of the ring pattern (e.g. "brrring-brrring") — repeats after
   * `loopGapMs` of silence. */
  chords: Chord[];
  noteMs: number;
  /** Silence between chords within one pass, in ms. */
  gapMs: number;
  /** Silence after one full pass before it repeats. */
  loopGapMs: number;
  peakGain: number;
}

export const RINGTONES: Record<RingtoneId, { label: string; description: string }> = {
  classic: {
    label: "Classic",
    description: "A traditional dual-tone telephone bell — brrring, brrring.",
  },
  chimes: {
    label: "Chimes",
    description: "A bright ascending four-note bell run.",
  },
  digital: {
    label: "Digital",
    description: "A crisp, modern electronic pulse.",
  },
  marimba: {
    label: "Marimba",
    description: "A warm, mellow descending marimba run.",
  },
  pulse: {
    label: "Pulse",
    description: "A short, urgent triple-beep.",
  },
};

const SPECS: Record<RingtoneId, RingtoneSpec> = {
  // Mimics the classic dual-frequency telephone bell: two simultaneous
  // tones, two short bursts ("ring-ring"), then a longer pause.
  classic: {
    chords: [
      { freqs: [480, 620], type: "sine" },
      { freqs: [480, 620], type: "sine" },
    ],
    noteMs: 380,
    gapMs: 200,
    loopGapMs: 900,
    peakGain: 0.2,
  },
  // Bright ascending triad+octave (C5 E5 G5 C6), triangle wave — a cheerful
  // bell run distinct from `notificationSound.ts`'s "classic" (which is a
  // quick 3-note pling, not a loop) and from the voice-chime family.
  chimes: {
    chords: [
      { freqs: [523.25], type: "triangle" },
      { freqs: [659.25], type: "triangle" },
      { freqs: [783.99], type: "triangle" },
      { freqs: [1046.5], type: "triangle" },
    ],
    noteMs: 130,
    gapMs: 20,
    loopGapMs: 550,
    peakGain: 0.18,
  },
  // Modern smartphone-like staccato electronic pulse — square wave,
  // alternating high/low, quick and attention-grabbing without being a
  // straight repeated beep like "pulse" below.
  digital: {
    chords: [
      { freqs: [880], type: "square" },
      { freqs: [880], type: "square" },
      { freqs: [660], type: "square" },
    ],
    noteMs: 90,
    gapMs: 60,
    loopGapMs: 420,
    peakGain: 0.12,
  },
  // Warm, mellow descending run (C5 G4 E4 C4), sine — the calmest of the
  // five, for anyone who finds the others too sharp.
  marimba: {
    chords: [
      { freqs: [523.25], type: "sine" },
      { freqs: [392.0], type: "sine" },
      { freqs: [329.63], type: "sine" },
      { freqs: [261.63], type: "sine" },
    ],
    noteMs: 150,
    gapMs: 30,
    loopGapMs: 650,
    peakGain: 0.22,
  },
  // A short, urgent triple-beep on a single pitch — sawtooth, the most
  // insistent-sounding of the five.
  pulse: {
    chords: [
      { freqs: [660], type: "sawtooth" },
      { freqs: [660], type: "sawtooth" },
      { freqs: [660], type: "sawtooth" },
    ],
    noteMs: 70,
    gapMs: 70,
    loopGapMs: 500,
    peakGain: 0.14,
  },
};

let audioCtx: AudioContext | null = null;

function getContext(): AudioContext {
  audioCtx ??= new AudioContext();
  return audioCtx;
}

/** Plays one pass of `spec` starting now; returns its total duration in ms
 * (including internal gaps, not `loopGapMs`) so the caller knows when to
 * schedule the next pass. */
function playPass(spec: RingtoneSpec): number {
  const ctx = getContext();
  const start0 = ctx.currentTime;
  const step = (spec.noteMs + spec.gapMs) / 1000;

  spec.chords.forEach((chord, i) => {
    const start = start0 + i * step;
    const end = start + spec.noteMs / 1000;
    chord.freqs.forEach((freq) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = chord.type;
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(spec.peakGain, start + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, end);
      osc.connect(gain).connect(ctx.destination);
      osc.start(start);
      osc.stop(end + 0.02);
    });
  });

  return spec.chords.length * (spec.noteMs + spec.gapMs);
}

export interface RingtoneHandle {
  stop: () => void;
}

/** Shared scheduling loop behind `startRingtone`/`previewRingtone`/
 * `startCallingSound`: plays `spec` on repeat until `.stop()` is called, or
 * — if `maxCycles` is given — until it's played that many times, whichever
 * comes first. */
function loopSpec(spec: RingtoneSpec, maxCycles?: number): RingtoneHandle {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let played = 0;

  const cycle = () => {
    if (stopped) return;
    played += 1;
    const passMs = playPass(spec);
    if (maxCycles === undefined || played < maxCycles) {
      timer = setTimeout(cycle, passMs + spec.loopGapMs);
    }
  };
  cycle();

  return {
    stop: () => {
      stopped = true;
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    },
  };
}

/** Starts looping `soundId` until `.stop()` is called — used for the actual
 * incoming-call ring (looped for as long as the call is ringing). */
export function startRingtone(soundId: RingtoneId = "classic"): RingtoneHandle {
  return loopSpec(SPECS[soundId] ?? SPECS.classic);
}

/** Plays `soundId` for a couple of loops then stops itself — Settings'
 * preview button. Returns a handle too, so picking a different tone (or
 * closing Settings) mid-preview can cut it off early. */
export function previewRingtone(soundId: RingtoneId = "classic", cycles = 2): RingtoneHandle {
  return loopSpec(SPECS[soundId] ?? SPECS.classic, cycles);
}

// A soft, steady dual-tone pulse — not one of the five `RINGTONES` above and
// not user-selectable: real telephony doesn't let the caller pick what
// their own ringback sounds like either, and mixing it into the same
// picker as the callee's ringtone would conflate two different sounds for
// two different people.
const CALLING_SPEC: RingtoneSpec = {
  chords: [{ freqs: [400, 450], type: "sine" }],
  noteMs: 1000,
  gapMs: 0,
  loopGapMs: 1200,
  peakGain: 0.12,
};

/** Starts the caller's own "ringback" — played from the moment a call is
 * placed until it's answered, declined, ends, or fails, mirroring
 * `startRingtone` on the callee's side. */
export function startCallingSound(): RingtoneHandle {
  return loopSpec(CALLING_SPEC);
}
