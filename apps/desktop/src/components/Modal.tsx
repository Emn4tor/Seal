import type { FormEvent } from "react";
import { useState } from "react";
import { useModalEscape } from "../lib/useModalEscape";

interface ModalProps {
  title: string;
  description?: string;
  fieldLabel: string;
  placeholder: string;
  submitLabel: string;
  onSubmit: (value: string) => Promise<void>;
  onClose: () => void;
  monospaceInput?: boolean;
}

export function Modal({
  title,
  description,
  fieldLabel,
  placeholder,
  submitLabel,
  onSubmit,
  onClose,
  monospaceInput,
}: ModalProps) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useModalEscape(onClose);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!value.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(value.trim());
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
        <h2 className="font-display text-base font-semibold text-text">{title}</h2>
        {description && <p className="mt-1.5 text-[13px] leading-relaxed text-text-muted">{description}</p>}
        <form onSubmit={handleSubmit} className="mt-4">
          <label className="mb-1.5 block text-xs font-medium uppercase tracking-wider text-text-faint">
            {fieldLabel}
          </label>
          <input
            autoFocus
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={placeholder}
            className={`w-full rounded-md border border-border bg-ink px-3 py-2 text-sm text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none ${
              monospaceInput ? "font-mono" : ""
            }`}
          />
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
              disabled={!value.trim() || busy}
              className="rounded-md bg-brass px-3.5 py-1.5 text-sm font-medium text-ink transition enabled:hover:scale-105 enabled:hover:brightness-110 enabled:active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {busy ? "Working…" : submitLabel}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
