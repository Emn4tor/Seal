import type { FormEvent } from "react";
import { useState } from "react";
import type { ChannelKind } from "../lib/types";
import { useModalEscape } from "../lib/useModalEscape";

interface CreateChannelModalProps {
  /** Which kind was requested via the "+" that opened this (text or voice
   * section) — preselected, but still changeable before submitting. */
  initialKind: ChannelKind;
  onSubmit: (name: string, kind: ChannelKind) => Promise<void>;
  onClose: () => void;
}

export function CreateChannelModal({ initialKind, onSubmit, onClose }: CreateChannelModalProps) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ChannelKind>(initialKind);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useModalEscape(onClose);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(name.trim(), kind);
      onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="w-full max-w-sm rounded-xl border border-border bg-surface p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-display text-base font-semibold text-text">Create a channel</h2>
        <form onSubmit={handleSubmit} className="mt-4">
          <label className="mb-1.5 block text-xs font-medium uppercase tracking-wider text-text-faint">
            Channel name
          </label>
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. secret-plans"
            className="w-full rounded-md border border-border bg-ink px-3 py-2 text-sm text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none"
          />

          <label className="mb-1.5 mt-4 block text-xs font-medium uppercase tracking-wider text-text-faint">
            Channel type
          </label>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => setKind("text")}
              className={`flex-1 rounded-md border px-3 py-2 text-left text-sm transition hover:-translate-y-px active:translate-y-0 ${
                kind === "text" ? "border-brass bg-brass-wash text-text" : "border-border text-text-muted hover:border-brass-dim"
              }`}
            >
              <span className="mr-1.5 text-text-faint">#</span> Text
            </button>
            <button
              type="button"
              onClick={() => setKind("voice")}
              className={`flex-1 rounded-md border px-3 py-2 text-left text-sm transition hover:-translate-y-px active:translate-y-0 ${
                kind === "voice" ? "border-brass bg-brass-wash text-text" : "border-border text-text-muted hover:border-brass-dim"
              }`}
            >
              Voice
            </button>
          </div>
          {error && <p className="mt-2 text-xs text-danger">{error}</p>}
          <div className="mt-4 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md px-3 py-1.5 text-sm text-text-muted hover:bg-surface-raised"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!name.trim() || busy}
              className="rounded-md bg-brass px-3.5 py-1.5 text-sm font-medium text-ink transition enabled:hover:scale-105 enabled:hover:brightness-110 enabled:active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {busy ? "Creating…" : "Create channel"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
