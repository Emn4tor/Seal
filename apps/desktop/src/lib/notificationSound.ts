/** A short, synthesized chime for incoming messages — no audio asset file,
 * generated on the fly with the Web Audio API (same technique as
 * `voiceSounds.ts`, but its own distinct three-note "pling" in a higher
 * register so it doesn't get confused with the voice join/leave chimes). */

let audioCtx: AudioContext | null = null;

function getContext(): AudioContext {
  audioCtx ??= new AudioContext();
  return audioCtx;
}

/** A bright ascending triad (A5 → C#6 → E6) — deliberately quick and
 * bell-like rather than a flat single-tone "beep". */
export function playMessageSound() {
  const ctx = getContext();
  const start0 = ctx.currentTime;
  const notes = [880, 1108.73, 1318.51];
  const noteMs = 70;

  notes.forEach((freq, i) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "triangle";
    osc.frequency.value = freq;

    const start = start0 + i * (noteMs / 1000) * 0.85;
    const end = start + noteMs / 1000 + 0.12;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.18, start + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, end);

    osc.connect(gain).connect(ctx.destination);
    osc.start(start);
    osc.stop(end + 0.02);
  });
}
