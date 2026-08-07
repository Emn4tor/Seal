import { useEffect, useRef, useState } from "react";
import { api, onChatEvent, onMicMutedChanged } from "../lib/tauri";
import { getMicThresholdDb } from "../lib/voiceSettings";
import { getPttEnabled, getPttShortcut } from "../lib/pushToTalk";
import {
  playMicMutedSound,
  playMicUnmutedSound,
  playVoiceJoinSound,
  playVoiceLeaveSound,
} from "../lib/voiceSounds";

interface VoiceCallPanelProps {
  groupId: string;
  channelId: string;
  channelName: string;
  currentUserId: string;
  /** Whether *this* channel is the one currently joined — a controlled
   * prop rather than local state, owned by `App.tsx` alongside `activeCall`
   * (the 1:1 call state): the two are mutually exclusive server-side (see
   * `AppService::leave_any_current_call`), and keeping "which one is
   * active" in one shared place is what lets this panel notice — and
   * reset itself — when starting or accepting a 1:1 call elsewhere in the
   * app silently left this channel out from under it, instead of going on
   * showing a call that's already dead. */
  joined: boolean;
  onJoin: () => Promise<void>;
  onLeave: () => Promise<void>;
}

const SPEAKING_POLL_MS = 200;

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

function ChangerIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 12a8 8 0 0 1 13.66-5.66M20 12a8 8 0 0 1-13.66 5.66"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
      <path d="M17 3v3.5H13.5M7 21v-3.5h3.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function HearSelfIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="M11 5 6 9H3v6h3l5 4V5Z" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
      <path d="M16 9a4 4 0 0 1 0 6M18.5 7a7.5 7.5 0 0 1 0 10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function LeaveIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="m5 17 14-10M5 7l14 10" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

function ParticipantTile({ userId, isSelf, speaking }: { userId: string; isSelf: boolean; speaking: boolean }) {
  return (
    <div className="flex w-24 flex-col items-center gap-2">
      <div
        className={`flex h-16 w-16 items-center justify-center rounded-full bg-surface-raised font-display text-xl font-medium text-text ring-2 transition-all ${
          speaking ? "ring-verdigris shadow-[0_0_0_5px_rgba(79,143,134,0.35)]" : "ring-verdigris-dim"
        }`}
      >
        {userId.slice(0, 1).toUpperCase()}
      </div>
      <p className="max-w-full truncate text-xs text-text-muted">{isSelf ? "You" : `${userId.slice(0, 8)}…`}</p>
    </div>
  );
}

/**
 * Real-time voice: mesh-connects to every other participant over a raw
 * `libp2p-stream`, mixed and played back locally. The "one click" voice
 * changer is a best-effort pitch-shift disguise for casual listening — not
 * a rigorous anonymity guarantee against serious voice-biometric analysis,
 * and it's labeled that way here rather than oversold.
 */
export function VoiceCallPanel({
  groupId,
  channelId,
  channelName,
  currentUserId,
  joined,
  onJoin,
  onLeave,
}: VoiceCallPanelProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [participants, setParticipants] = useState<string[]>([]);
  const [muted, setMuted] = useState(false);
  const [changerEnabled, setChangerEnabled] = useState(false);
  const [hearSelf, setHearSelf] = useState(false);
  const [speaking, setSpeaking] = useState<Set<string>>(new Set());
  const participantsRef = useRef<string[]>([]);
  const pttEnabled = getPttEnabled() && !!getPttShortcut();

  useEffect(() => {
    const unlisten = onChatEvent((event) => {
      if (
        event.type === "voice_participants_changed" &&
        event.group_id === groupId &&
        event.channel_id === channelId
      ) {
        const before = participantsRef.current;
        const after = event.user_ids;
        if (after.some((id) => !before.includes(id))) playVoiceJoinSound();
        else if (before.some((id) => !after.includes(id))) playVoiceLeaveSound();
        participantsRef.current = after;
        setParticipants(after);
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [groupId, channelId]);

  // Polls rather than pushing events for every 20ms mixer tick — speaking
  // state only needs to be "current within a couple hundred ms" for a UI
  // indicator, not sample-accurate, and this keeps the backend from having
  // to know about frontend event plumbing for something this lightweight.
  useEffect(() => {
    if (!joined) return;
    const interval = setInterval(async () => {
      try {
        const ids = await api.getVoiceSpeakingParticipants();
        setSpeaking(new Set(ids));
      } catch {
        // call may have just ended — next poll (or unmount) settles it.
      }
    }, SPEAKING_POLL_MS);
    return () => clearInterval(interval);
  }, [joined]);

  // Only the tray menu's mic-mute item needs this — see `onMicMutedChanged`
  // — since it's the one way mute state can change with this panel's own
  // `muted` state having no idea it happened.
  useEffect(() => {
    if (!joined) return;
    const unlisten = onMicMutedChanged((newMuted) => {
      setMuted(newMuted);
      if (newMuted) playMicMutedSound();
      else playMicUnmutedSound();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [joined]);

  // The single place local UI state gets cleared for *any* reason we stop
  // being joined — clicking "Leave" ourselves, or `App.tsx` silently
  // auto-leaving this channel because a 1:1 call started/was accepted
  // elsewhere. Reacting to the prop rather than duplicating this in
  // `handleLeave` means both paths always leave the same clean slate
  // behind, instead of the "taken over elsewhere" path skipping it.
  useEffect(() => {
    if (joined) return;
    setParticipants([]);
    participantsRef.current = [];
    setSpeaking(new Set());
    setMuted(false);
    setChangerEnabled(false);
    setHearSelf(false);
  }, [joined]);

  async function handleJoin() {
    setError(null);
    setBusy(true);
    try {
      await onJoin();
      await api.setMicThresholdDb(getMicThresholdDb());
      if (pttEnabled) await api.setMicMuted(true);
      // Asks the backend rather than assuming `pttEnabled` — it's the
      // actual source of truth, and it's what just got set two lines up.
      setMuted(await api.getMicMuted());
      playVoiceJoinSound();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleLeave() {
    setBusy(true);
    try {
      await onLeave();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
      playVoiceLeaveSound();
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
      // Back to what it actually is, otherwise the button can keep
      // showing "muted" (or not) while the backend call that was supposed
      // to make that true actually failed.
      setMuted(!next);
      setError(String(err));
    }
  }

  async function toggleChanger() {
    const next = !changerEnabled;
    setChangerEnabled(next);
    try {
      await api.setVoiceChangerEnabled(next);
    } catch (err) {
      setChangerEnabled(!next);
      setError(String(err));
    }
  }

  async function toggleHearSelf() {
    const next = !hearSelf;
    setHearSelf(next);
    try {
      await api.setHearSelf(next);
    } catch (err) {
      setHearSelf(!next);
      setError(String(err));
    }
  }

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col bg-ink">
      <div className="flex items-center gap-2.5 border-b border-border px-5 py-3.5">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" className="text-text-faint">
          <path d="M11 5 6 9H3v6h3l5 4V5Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
          <path d="M16 9a4 4 0 0 1 0 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
        <div className="min-w-0">
          <h2 className="truncate font-display text-[15px] font-semibold text-text">{channelName}</h2>
          <p className="truncate text-xs text-text-faint">
            {joined ? `${participants.length + 1} in this call` : "Voice channel"}
          </p>
        </div>
      </div>

      <div className="flex flex-1 flex-col items-center justify-center gap-6 px-6">
        {error && (
          <p className="max-w-sm rounded-lg border border-danger-dim bg-danger-wash px-3 py-2 text-center text-xs text-danger">
            {error}
          </p>
        )}

        {!joined ? (
          <>
            <p className="text-center text-sm text-text-faint">Nobody's talking yet — join in and say hello.</p>
            <button
              onClick={handleJoin}
              disabled={busy}
              className="flex items-center gap-2 rounded-xl bg-verdigris px-6 py-3 font-display text-sm font-semibold text-ink transition enabled:hover:scale-105 enabled:hover:bg-verdigris-dim enabled:active:scale-95 disabled:opacity-60"
            >
              <MicIcon muted={false} />
              {busy ? "Joining…" : "Join Voice"}
            </button>
          </>
        ) : (
          <div className="flex flex-wrap justify-center gap-6">
            <ParticipantTile userId={currentUserId} isSelf speaking={speaking.has(currentUserId)} />
            {participants.map((userId) => (
              <ParticipantTile key={userId} userId={userId} isSelf={false} speaking={speaking.has(userId)} />
            ))}
          </div>
        )}
      </div>

      {joined && (
        <div className="flex flex-wrap items-center justify-center gap-3 border-t border-border px-5 py-4">
          {pttEnabled ? (
            <div className="flex items-center gap-2 rounded-full bg-surface-raised px-4 py-2.5 text-sm text-text-muted">
              <MicIcon muted={false} />
              Push to talk: hold <span className="font-mono text-text">{getPttShortcut()}</span>
            </div>
          ) : (
            <button
              onClick={toggleMuted}
              aria-pressed={muted}
              title={muted ? "Unmute" : "Mute"}
              className={`flex h-11 w-11 items-center justify-center rounded-full transition hover:scale-105 active:scale-95 ${
                muted ? "bg-danger-wash text-danger" : "bg-surface-raised text-text hover:bg-surface"
              }`}
            >
              <MicIcon muted={muted} />
            </button>
          )}
          <button
            onClick={toggleChanger}
            aria-pressed={changerEnabled}
            title="Voice changer: a best-effort pitch-shift disguise, not a rigorous anonymity guarantee"
            className={`flex items-center gap-2 rounded-full px-4 py-2.5 text-sm transition hover:scale-105 active:scale-95 ${
              changerEnabled ? "bg-brass-wash text-brass" : "bg-surface-raised text-text hover:bg-surface"
            }`}
          >
            <ChangerIcon />
            Voice changer
          </button>
          <button
            onClick={toggleHearSelf}
            aria-pressed={hearSelf}
            title="Hear yourself: loop your own mic back to your speakers"
            className={`flex items-center gap-2 rounded-full px-4 py-2.5 text-sm transition hover:scale-105 active:scale-95 ${
              hearSelf ? "bg-brass-wash text-brass" : "bg-surface-raised text-text hover:bg-surface"
            }`}
          >
            <HearSelfIcon />
            Hear yourself
          </button>
          <button
            onClick={handleLeave}
            disabled={busy}
            title="Leave voice channel"
            className="flex h-11 w-11 items-center justify-center rounded-full bg-danger-wash text-danger transition enabled:hover:scale-105 enabled:hover:bg-danger-dim enabled:hover:text-ink enabled:active:scale-95 disabled:opacity-60"
          >
            <LeaveIcon />
          </button>
        </div>
      )}
      {joined && (
        <p className="px-5 pb-3 text-center text-[11px] text-text-faint">
          Voice changer disguises pitch for casual listening — it isn't a guarantee against serious voice-analysis.
        </p>
      )}
    </div>
  );
}
