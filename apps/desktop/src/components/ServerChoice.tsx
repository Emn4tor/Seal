import { useEffect, useState } from "react";
import { EMBEDDED_SERVER_SENTINEL, api } from "../lib/tauri";
import { CipherSeal } from "./CipherSeal";

interface ServerChoiceProps {
  onChosen: (serverUrl: string) => Promise<void>;
}

type Mode = "seal" | "custom" | "local";

export function ServerChoice({ onChosen }: ServerChoiceProps) {
  // `undefined` while loading, `null` once loaded if no official server is
  // configured in this build (see README — `SEAL_DEFAULT_DIRECTORY_URL`).
  const [officialUrl, setOfficialUrl] = useState<string | null | undefined>(undefined);
  const [mode, setMode] = useState<Mode>("local");
  const [customUrl, setCustomUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getOfficialServerUrl().then((url) => {
      setOfficialUrl(url);
      if (url) setMode("seal");
    });
  }, []);

  const sealConfigured = Boolean(officialUrl);

  async function handleContinue() {
    const chosen =
      mode === "seal" ? officialUrl! : mode === "local" ? EMBEDDED_SERVER_SENTINEL : customUrl.trim();
    if (!chosen || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onChosen(chosen);
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center bg-ink px-6">
      <div className="w-full max-w-md">
        <div className="mb-8 flex items-center gap-3">
          <CipherSeal status="idle" size={28} />
          <span className="font-display text-lg font-semibold tracking-tight text-text">Seal</span>
        </div>

        <h1 className="font-display text-2xl font-semibold text-text">Choose a directory server</h1>
        <p className="mt-2 text-sm leading-relaxed text-text-muted">
          This is the one server Seal talks to, it only ever helps you find people by ID,
          never your messages. Anyone can run one; use Seal's network, or point at one you
          or someone you trust hosts.
        </p>

        <div className="mt-6 flex flex-col gap-3">
          <button
            onClick={() => sealConfigured && setMode("seal")}
            disabled={!sealConfigured}
            className={`rounded-lg border px-4 py-3 text-left transition ${
              mode === "seal"
                ? "border-brass bg-brass-wash"
                : "border-border bg-surface enabled:hover:-translate-y-px enabled:hover:border-brass-dim"
            } ${!sealConfigured ? "cursor-not-allowed opacity-40" : ""}`}
          >
            <p className="text-sm font-medium text-text">Seal</p>
            <p className="mt-0.5 font-mono text-xs text-text-faint">
              {officialUrl === undefined
                ? "Loading…"
                : sealConfigured
                  ? officialUrl
                  : "Not set up in this build yet, see the README."}
            </p>
          </button>

          <button
            onClick={() => setMode("custom")}
            className={`rounded-lg border px-4 py-3 text-left transition ${
              mode === "custom"
                ? "border-brass bg-brass-wash"
                : "border-border bg-surface hover:-translate-y-px hover:border-brass-dim"
            }`}
          >
            <p className="text-sm font-medium text-text">Custom server</p>
            <p className="mt-0.5 text-xs text-text-faint">Point at one you or someone else is hosting.</p>
            {mode === "custom" && (
              <input
                autoFocus
                value={customUrl}
                onChange={(e) => setCustomUrl(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                placeholder="https://directory.example.com"
                className="mt-2.5 w-full rounded-md border border-border bg-ink px-3 py-2 font-mono text-[13px] text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none"
              />
            )}
          </button>
        </div>

        <p className="mt-4 mb-1.5 text-[11px] uppercase tracking-wider text-text-faint">Or, just for testing</p>
        <button
          onClick={() => setMode("local")}
          className={`flex w-full items-center justify-between rounded-md border px-3 py-1.5 text-left transition ${
            mode === "local" ? "border-brass bg-brass-wash" : "border-border/60 bg-surface/60 hover:border-brass-dim"
          }`}
        >
          <span className="text-xs font-medium text-text-muted">Local test server</span>
          <span className="text-[11px] text-text-faint">Runs on this device only</span>
        </button>

        {error && <p className="mt-3 text-sm text-danger">{error}</p>}

        <button
          onClick={handleContinue}
          disabled={busy || (mode === "custom" && !customUrl.trim()) || (mode === "seal" && !officialUrl)}
          className="mt-6 w-full rounded-md bg-brass py-2.5 text-[15px] font-medium text-ink transition enabled:hover:scale-[1.02] enabled:hover:brightness-110 enabled:active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"
        >
          {busy ? "Connecting…" : "Continue"}
        </button>
        <p className="mt-3 text-xs text-text-faint">
          You can change this later in Settings, it takes effect the next time you start Seal.
        </p>
      </div>
    </div>
  );
}
