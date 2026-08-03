import type { FormEvent } from "react";
import { useState } from "react";
import { CipherSeal } from "./CipherSeal";
import { getAutostartEnabled, saveAutostartEnabled, syncAutostart } from "../lib/autostart";

interface OnboardingProps {
  /** "first" is this device's very first account; "add" is an additional
   * one created from the picker or Settings — same form, different copy. */
  mode?: "first" | "add";
  onSubmit: (displayName: string) => Promise<void>;
  onCancel?: () => void;
}

const promises = [
  {
    title: "Locked before it leaves your device",
    body: "Every message is sealed with your keys the instant you hit send. Group messages work the same way, with a key shared only between members.",
  },
  {
    title: "No inbox to hand over",
    body: "There's no server holding your conversations. Messages travel straight to the people in them, or not at all.",
  },
  {
    title: "The directory sees a name and an address",
    body: "That's the entire job of the one small server this app talks to — helping people find each other. It cannot read a single word you write.",
  },
];

export function Onboarding({ mode = "first", onSubmit, onCancel }: OnboardingProps) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autostart, setAutostart] = useState(getAutostartEnabled);
  const isAdd = mode === "add";

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(name.trim());
      if (!isAdd) {
        saveAutostartEnabled(autostart);
        syncAutostart(autostart);
      }
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center bg-ink px-6">
      <div className="grid w-full max-w-4xl grid-cols-1 gap-12 md:grid-cols-2">
        <div>
          <div className="mb-8 flex items-center gap-3">
            <CipherSeal status="secure" size={30} />
            <span className="font-display text-lg font-semibold tracking-tight text-text">Seal</span>
          </div>

          {isAdd ? (
            <h1 className="font-display text-4xl font-semibold leading-[1.1] tracking-tight text-text md:text-5xl">
              A fresh identity.
              <br />
              Fully separate.
            </h1>
          ) : (
            <h1 className="font-display text-4xl font-semibold leading-[1.1] tracking-tight text-text md:text-5xl">
              Nobody&rsquo;s listening.
              <br />
              Not even us.
            </h1>
          )}
          <p className="mt-5 max-w-sm text-[15px] leading-relaxed text-text-muted">
            {isAdd
              ? "This account gets its own keys, contacts, and messages — entirely separate from every other account on this device."
              : "Seal encrypts every conversation end to end, peer to peer. Read what that actually means below, then pick a name and get in."}
          </p>

          <form onSubmit={handleSubmit} className="mt-10 max-w-sm">
            <label htmlFor="display-name" className="mb-2 block text-xs font-medium uppercase tracking-wider text-text-faint">
              Display name
            </label>
            <input
              id="display-name"
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="What should people call you?"
              className="w-full rounded-md border border-border bg-surface px-3.5 py-2.5 text-[15px] text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none"
            />

            {!isAdd && (
              <div className="mt-4 flex items-center justify-between gap-3 rounded-md border border-border bg-surface px-3.5 py-3">
                <div>
                  <p className="text-sm font-medium text-text">Launch Seal at login</p>
                  <p className="mt-1 text-xs text-text-muted">
                    Starts automatically when you log into this device.
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setAutostart((v) => !v)}
                  aria-pressed={autostart}
                  className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-medium transition active:scale-90 ${
                    autostart ? "bg-verdigris-wash text-verdigris" : "bg-surface-raised text-text-muted"
                  }`}
                >
                  {autostart ? "On" : "Off"}
                </button>
              </div>
            )}

            {error && <p className="mt-2 text-sm text-danger">{error}</p>}
            <button
              type="submit"
              disabled={!name.trim() || busy}
              className="mt-4 w-full rounded-md bg-brass py-2.5 text-[15px] font-medium text-ink transition enabled:hover:scale-[1.02] enabled:hover:brightness-110 enabled:active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"
            >
              {busy ? "Generating your keys…" : isAdd ? "Add account" : "Get started"}
            </button>
            {isAdd && onCancel && (
              <button
                type="button"
                onClick={onCancel}
                disabled={busy}
                className="mt-2 w-full rounded-md py-2 text-sm text-text-muted hover:text-text disabled:cursor-not-allowed disabled:opacity-40"
              >
                Cancel
              </button>
            )}
            <p className="mt-3 text-xs text-text-faint">
              This creates a private key that never leaves this device. There&rsquo;s no
              password to remember and no account to recover.
            </p>
          </form>
        </div>

        <div className="flex flex-col justify-center gap-7 border-l border-border pl-10 max-md:hidden">
          {promises.map((p) => (
            <div key={p.title} className="flex gap-3.5">
              <CipherSeal status="idle" size={18} className="mt-1 shrink-0" />
              <div>
                <h3 className="font-display text-[15px] font-medium text-text">{p.title}</h3>
                <p className="mt-1 text-sm leading-relaxed text-text-muted">{p.body}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
