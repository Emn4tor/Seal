import { useEffect, useState } from "react";
import { CipherSeal } from "./CipherSeal";
import { api } from "../lib/tauri";
import { decodePairingOffer, type AccountSummary, type PairingOffer } from "../lib/types";

interface JoinViaPairingProps {
  serverUrl: string;
  onJoined: (account: AccountSummary) => Promise<void>;
  onCancel: () => void;
}

/** Tauri IPC failures often reject with a plain object, not a real
 * `Error` — an unguarded `String(err)` gives the unhelpful "[object Object]". */
function scanErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === "string" && obj.message) return obj.message;
    if (typeof obj.error === "string" && obj.error) return obj.error;
    try {
      const json = JSON.stringify(obj);
      if (json && json !== "{}") return json;
    } catch {
      // fall through to the generic message below
    }
  }
  return "Couldn't reach the camera. Try again.";
}

/** The joining side of QR pairing: scan another device's code (mobile) or
 * paste its JSON directly (works everywhere) — either way calls `api.joinViaPairing`. */
export function JoinViaPairing({ serverUrl, onJoined, onCancel }: JoinViaPairingProps) {
  const [pasted, setPasted] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);

  // `windowed: true` draws the camera preview *behind* the webview, so
  // the page must go transparent for exactly as long as a scan is active.
  useEffect(() => {
    if (!scanning) return;
    document.body.classList.add("qr-scan-active");
    return () => {
      document.body.classList.remove("qr-scan-active");
    };
  }, [scanning]);

  async function joinWithOffer(offer: PairingOffer) {
    setBusy(true);
    setError(null);
    try {
      const account = await api.joinViaPairing(serverUrl, offer);
      await onJoined(account);
    } catch (err) {
      setError(scanErrorMessage(err));
      setBusy(false);
    }
  }

  async function handleScan() {
    setError(null);
    setScanning(true);
    try {
      // Mobile-only (iOS/Android) — not implemented on desktop, so this
      // is expected to fail there; the paste fallback below covers that
      // case (and doubles as how desktop-to-desktop pairing is tested).
      const {
        scan,
        cancel: cancelScan,
        Format,
        checkPermissions,
        requestPermissions,
      } = await import("@tauri-apps/plugin-barcode-scanner");
      // Checked before calling `scan()` rather than letting it request
      // implicitly — that raced the OS prompt and failed the first try.
      let permission = await checkPermissions();
      if (permission !== "granted") {
        permission = await requestPermissions();
      }
      if (permission !== "granted") {
        setError(
          "Camera access is off for Seal — enable it in your device's settings to scan a QR code.",
        );
        return;
      }
      const result = await scan({ formats: [Format.QRCode], windowed: true });
      await cancelScan().catch(() => {});
      await joinWithOffer(decodePairingOffer(result.content));
    } catch (err) {
      setError(scanErrorMessage(err));
    } finally {
      setScanning(false);
    }
  }

  async function handleCancelScan() {
    try {
      const { cancel: cancelScan } = await import("@tauri-apps/plugin-barcode-scanner");
      await cancelScan();
    } catch {
      // Best-effort — the overlay comes down either way.
    }
    setScanning(false);
  }

  function handlePasteSubmit() {
    setError(null);
    try {
      joinWithOffer(decodePairingOffer(pasted));
    } catch {
      setError("That doesn't look like a valid pairing code.");
    }
  }

  if (scanning) {
    return (
      <div className="fixed inset-0 z-50 flex flex-col items-center justify-center bg-transparent px-6">
        {/* Huge box-shadow spread dims outside the square without going
            fully opaque, so the camera feed behind stays visible. */}
        <div className="h-64 w-64 rounded-2xl border-2 border-brass shadow-[0_0_0_9999px_rgba(14,17,22,0.55)]" />
        <p className="mt-6 text-sm font-medium text-text drop-shadow">Point your camera at the QR code</p>
        <button
          onClick={handleCancelScan}
          className="mt-6 rounded-md border border-border bg-surface px-4 py-2 text-sm font-medium text-text transition hover:-translate-y-px hover:border-brass-dim"
        >
          Cancel
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full items-center justify-center bg-ink px-6">
      <div className="w-full max-w-md">
        <div className="mb-8 flex items-center gap-3">
          <CipherSeal status="secure" size={30} />
          <span className="font-display text-lg font-semibold tracking-tight text-text">Seal</span>
        </div>

        <h1 className="font-display text-2xl font-semibold text-text">Join with a QR code</h1>
        <p className="mt-2 text-[15px] text-text-muted">
          Open Seal on your other device, go to Settings → Devices, and show the QR code there.
        </p>

        <button
          onClick={handleScan}
          disabled={busy || scanning}
          className="mt-6 w-full rounded-md bg-brass py-2.5 text-[15px] font-medium text-ink transition enabled:hover:scale-[1.02] enabled:hover:brightness-110 enabled:active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"
        >
          {scanning ? "Scanning…" : "Scan with camera"}
        </button>

        <div className="mt-6">
          <label htmlFor="pairing-paste" className="mb-2 block text-xs font-medium uppercase tracking-wider text-text-faint">
            Or paste the pairing code
          </label>
          <textarea
            id="pairing-paste"
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
            rows={4}
            placeholder="Paste the code shown under the QR"
            className="w-full resize-none rounded-md border border-border bg-surface px-3.5 py-2.5 font-mono text-xs text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none"
          />
          <button
            onClick={handlePasteSubmit}
            disabled={busy || !pasted.trim()}
            className="mt-2 w-full rounded-md border border-border px-3.5 py-2 text-sm font-medium text-brass transition enabled:hover:-translate-y-px enabled:hover:border-brass-dim enabled:hover:bg-brass-wash disabled:cursor-not-allowed disabled:opacity-40"
          >
            {busy ? "Joining…" : "Join"}
          </button>
        </div>

        {error && <p className="mt-3 text-sm text-danger">{error}</p>}

        <button
          onClick={onCancel}
          disabled={busy}
          className="mt-4 w-full rounded-md py-2 text-sm text-text-muted hover:text-text disabled:cursor-not-allowed disabled:opacity-40"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
