import { useEffect, useState } from "react";
import { api, onMicMutedChanged } from "../lib/tauri";
import { getMicThresholdDb } from "../lib/voiceSettings";
import { getPttEnabled, getPttShortcut } from "../lib/pushToTalk";
import { playMicMutedSound, playMicUnmutedSound } from "../lib/voiceSounds";

export type CallPhase = "outgoing" | "incoming" | "connected";

export interface ActiveCallInfo {
  callId: string;
  peerUserId: string;
  phase: CallPhase;
  /** True only for an outgoing call whose `callId` is still a local
   * placeholder — set the instant the user presses "Call", before
   * `callContact`'s directory lookup + dial has actually resolved with a
   * real id from the backend. Lets the ring screen (and ringback sound)
   * show up immediately instead of waiting out however long the dial
   * takes to fail or succeed. Never set for an incoming or connected call. */
  pending?: boolean;
}

interface CallOverlayProps {
  call: ActiveCallInfo;
  peerDisplayName: string;
  /** May throw — on failure the call stays in the "incoming" phase (with
   * the error shown) so the user can retry or fall back to declining,
   * rather than the whole overlay silently vanishing. */
  onAccept: () => Promise<void>;
  onDecline: () => void;
  onEnd: () => void;
}

function PhoneIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path
        d="M6.6 10.8c1.4 2.8 3.8 5.2 6.6 6.6l2.2-2.2a1 1 0 0 1 1.1-.2c1.2.5 2.5.7 3.8.7a1 1 0 0 1 1 1V20a1 1 0 0 1-1 1C10.5 21 3 13.5 3 4.7a1 1 0 0 1 1-1h3.3a1 1 0 0 1 1 1c0 1.3.2 2.6.7 3.8a1 1 0 0 1-.2 1.1L6.6 10.8Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function EndCallIcon({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" style={{ transform: "rotate(135deg)" }}>
      <path
        d="M3 13c5-4.5 13-4.5 18 0l-2.6 3a1 1 0 0 1-1.3.1l-2-1.5a1 1 0 0 0-1.2 0c-.9.7-2.3.7-3.2 0a1 1 0 0 0-1.2 0l-2 1.5a1 1 0 0 1-1.3-.1L3 13Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function MicIcon({ muted }: { muted: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path
        d="M12 15a3.5 3.5 0 0 0 3.5-3.5v-5a3.5 3.5 0 0 0-7 0v5A3.5 3.5 0 0 0 12 15Z"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path d="M6 11a6 6 0 0 0 12 0M12 17v3.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      {muted && <path d="M4 4l16 16" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />}
    </svg>
  );
}

function Avatar({ userId, size, pulse }: { userId: string; size: number; pulse?: boolean }) {
  return (
    <div
      style={{ width: size, height: size, fontSize: size * 0.4 }}
      className={`flex shrink-0 items-center justify-center rounded-full bg-surface-raised font-display font-medium text-text ring-2 ring-verdigris-dim ${
        pulse ? "animate-pulse" : ""
      }`}
    >
      {userId.slice(0, 1).toUpperCase()}
    </div>
  );
}

function formatDuration(totalSeconds: number): string {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/**
 * The 1:1 counterpart to `VoiceCallPanel` — logic-wise the same call
 * underneath (same `VoiceCallState`, same mute/threshold plumbing), but a
 * 1:1 call only ever has one other participant, so the UI leans into that:
 * a full-screen ring screen while it's ringing (either direction) — a call
 * demands a decision, so it takes over — collapsing to a small persistent
 * pill once connected, so a call doesn't block browsing the rest of Seal
 * the way a full voice-channel panel does.
 */
export function CallOverlay({ call, peerDisplayName, onAccept, onDecline, onEnd }: CallOverlayProps) {
  const [muted, setMuted] = useState(false);
  const [actionPending, setActionPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const pttEnabled = getPttEnabled() && !!getPttShortcut();

  useEffect(() => {
    setError(null);
  }, [call.phase]);

  // Ticks the call-duration clock once actually connected, and (re)applies
  // this device's mic preferences — a fresh `VoiceCallState` always starts
  // at the hardcoded default threshold/unmuted, same as `VoiceCallPanel`'s
  // `handleJoin` — needed here too since, on the caller's side, audio
  // starts server-side the moment the callee accepts, with no local
  // "join" call of our own to hang this off of.
  useEffect(() => {
    if (call.phase !== "connected") return;
    setElapsed(0);
    const interval = setInterval(() => setElapsed((s) => s + 1), 1000);

    api.setMicThresholdDb(getMicThresholdDb()).catch(() => {});
    (async () => {
      if (pttEnabled) await api.setMicMuted(true).catch(() => {});
      setMuted(await api.getMicMuted().catch(() => false));
    })();

    const unlisten = onMicMutedChanged((newMuted) => {
      setMuted(newMuted);
      if (newMuted) playMicMutedSound();
      else playMicUnmutedSound();
    });
    return () => {
      clearInterval(interval);
      unlisten.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [call.callId, call.phase]);

  async function handleAccept() {
    setError(null);
    setActionPending(true);
    try {
      await onAccept();
    } catch (err) {
      setError(String(err));
    } finally {
      setActionPending(false);
    }
  }

  async function toggleMuted() {
    const next = !muted;
    setMuted(next);
    if (next) playMicMutedSound();
    else playMicUnmutedSound();
    try {
      await api.setMicMuted(next);
    } catch (err) {
      setMuted(!next);
      setError(String(err));
    }
  }

  if (call.phase === "connected") {
    return (
      <div className="fixed bottom-5 right-5 z-40 flex items-center gap-3 rounded-2xl border border-border bg-surface/95 px-4 py-3 shadow-lg backdrop-blur">
        <Avatar userId={call.peerUserId} size={40} />
        <div className="min-w-0">
          <p className="max-w-[140px] truncate text-sm font-medium text-text">{peerDisplayName}</p>
          <p className="font-mono text-xs text-text-faint">{formatDuration(elapsed)}</p>
          {error && <p className="max-w-[160px] truncate text-[10px] text-danger">{error}</p>}
        </div>
        {pttEnabled ? (
          <div
            title={`Push to talk: hold ${getPttShortcut()}`}
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface-raised text-text-muted"
          >
            <MicIcon muted={false} />
          </div>
        ) : (
          <button
            onClick={toggleMuted}
            aria-pressed={muted}
            title={muted ? "Unmute" : "Mute"}
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition hover:scale-105 active:scale-95 ${
              muted ? "bg-danger-wash text-danger" : "bg-surface-raised text-text hover:bg-surface"
            }`}
          >
            <MicIcon muted={muted} />
          </button>
        )}
        <button
          onClick={onEnd}
          title="Hang up"
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-danger text-ink transition hover:scale-105 hover:brightness-110 active:scale-95"
        >
          <EndCallIcon size={18} />
        </button>
      </div>
    );
  }

  const isIncoming = call.phase === "incoming";
  return (
    <div className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-10 bg-ink/97 backdrop-blur-sm">
      <div className="flex flex-col items-center gap-4">
        <Avatar userId={call.peerUserId} size={112} pulse />
        <div className="text-center">
          <p className="font-display text-2xl font-semibold text-text">{peerDisplayName}</p>
          <p className="mt-1.5 text-sm text-text-faint">{isIncoming ? "Incoming call…" : "Calling…"}</p>
        </div>
      </div>

      {error && <p className="max-w-sm text-center text-sm text-danger">{error}</p>}

      <div className="flex items-center gap-8">
        {isIncoming ? (
          <>
            <div className="flex flex-col items-center gap-2">
              <button
                onClick={onDecline}
                disabled={actionPending}
                title="Decline"
                className="flex h-16 w-16 items-center justify-center rounded-full bg-danger text-ink transition enabled:hover:scale-105 enabled:hover:brightness-110 enabled:active:scale-95 disabled:opacity-60"
              >
                <EndCallIcon />
              </button>
              <span className="text-xs text-text-faint">Decline</span>
            </div>
            <div className="flex flex-col items-center gap-2">
              <button
                onClick={handleAccept}
                disabled={actionPending}
                title="Accept"
                className="flex h-16 w-16 items-center justify-center rounded-full bg-verdigris text-ink transition enabled:hover:scale-105 enabled:hover:bg-verdigris-dim enabled:active:scale-95 disabled:opacity-60"
              >
                <PhoneIcon size={26} />
              </button>
              <span className="text-xs text-text-faint">{actionPending ? "Answering…" : "Accept"}</span>
            </div>
          </>
        ) : (
          <div className="flex flex-col items-center gap-2">
            <button
              onClick={onEnd}
              disabled={actionPending}
              title="Cancel"
              className="flex h-16 w-16 items-center justify-center rounded-full bg-danger text-ink transition enabled:hover:scale-105 enabled:hover:brightness-110 enabled:active:scale-95 disabled:opacity-60"
            >
              <EndCallIcon />
            </button>
            <span className="text-xs text-text-faint">Cancel</span>
          </div>
        )}
      </div>
    </div>
  );
}
