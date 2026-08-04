/** Short, synthesized two-note chimes for voice-channel join/leave — no
 * audio asset files, generated on the fly with the Web Audio API. */

let audioCtx: AudioContext | null = null;

function getContext(): AudioContext {
  audioCtx ??= new AudioContext();
  return audioCtx;
}

function playTone(frequencies: number[], noteMs: number) {
  const ctx = getContext();
  const start0 = ctx.currentTime;
  frequencies.forEach((freq, i) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = freq;
    const start = start0 + i * (noteMs / 1000);
    const end = start + noteMs / 1000;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.22, start + 0.012);
    gain.gain.exponentialRampToValueAtTime(0.0001, end);
    osc.connect(gain).connect(ctx.destination);
    osc.start(start);
    osc.stop(end + 0.02);
  });
}

/** Rising two-note chime — played both when you join and when someone else does. */
export function playVoiceJoinSound() {
  playTone([440, 660], 90);
}

/** Falling two-note chime — played both when you leave and when someone else does. */
export function playVoiceLeaveSound() {
  playTone([660, 440], 90);
}

/** A single short, quiet blip for your own mic mute/unmute — deliberately
 * not a two-note chime like join/leave: this can fire many times in quick
 * succession (toggling back and forth), so it stays brief and low-key
 * rather than announcing itself. An octave apart (370 Hz / 740 Hz) so
 * "on"/"off" are unambiguous by ear alone. Only for an explicit toggle
 * (the in-call mute button, the tray menu item) — never for push-to-talk's
 * per-keypress mute/unmute, which would turn this into constant noise. */
export function playMicMutedSound() {
  playTone([370], 70);
}

export function playMicUnmutedSound() {
  playTone([740], 70);
}
