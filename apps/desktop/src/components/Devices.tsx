import { useCallback, useEffect, useState } from "react";
import QRCode from "qrcode";
import { api, onChatEvent } from "../lib/tauri";
import { encodePairingOffer, type Device, type PairingOffer } from "../lib/types";

function shortId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 12)}…` : id;
}

/** The "Devices" settings tab: shows every device registered to this
 * account with a "Sync" button, and lets this device offer a QR code for
 * a new device to scan and join the account. */
export function Devices() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [pairingBusy, setPairingBusy] = useState(false);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [secondsLeft, setSecondsLeft] = useState(0);
  const [codeCopied, setCodeCopied] = useState(false);

  const [syncingDeviceId, setSyncingDeviceId] = useState<string | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [lastSyncedDeviceId, setLastSyncedDeviceId] = useState<string | null>(null);

  const refreshDevices = useCallback(async () => {
    try {
      setDevices(await api.listMyDevices());
      setLoadError(null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshDevices();
  }, [refreshDevices]);

  // A sync's result arrives asynchronously as a `chat-event`, not as the
  // direct result of `syncWithDevice` (which only confirms the *request*
  // went out) — see `AppService::sync_with_device`'s doc comment.
  useEffect(() => {
    let cancelled = false;
    const unlisten = onChatEvent((event) => {
      if (event.type !== "sync_completed" || cancelled) return;
      setSyncingDeviceId((current) => (current === event.device_id ? null : current));
      setLastSyncedDeviceId(event.device_id);
      refreshDevices();
    });
    return () => {
      cancelled = true;
      unlisten.then((f) => f());
    };
  }, [refreshDevices]);

  // Counts down the currently-shown QR's remaining validity, clearing it
  // once expired so a stale, unscannable code doesn't linger on screen.
  useEffect(() => {
    if (!offer) return;
    const tick = () => {
      const remaining = Math.max(0, Math.round(offer.expires_at - Date.now() / 1000));
      setSecondsLeft(remaining);
      if (remaining === 0) {
        setOffer(null);
        setQrDataUrl(null);
      }
    };
    tick();
    const interval = setInterval(tick, 1000);
    return () => clearInterval(interval);
  }, [offer]);

  async function handleStartPairing() {
    setPairingBusy(true);
    setPairingError(null);
    try {
      const newOffer = await api.startPairing();
      setOffer(newOffer);
      setCodeCopied(false);
      setQrDataUrl(await QRCode.toDataURL(encodePairingOffer(newOffer), { margin: 1, width: 260 }));
    } catch (err) {
      setPairingError(String(err));
    } finally {
      setPairingBusy(false);
    }
  }

  async function handleSync(deviceId: string) {
    setSyncingDeviceId(deviceId);
    setSyncError(null);
    try {
      await api.syncWithDevice(deviceId);
    } catch (err) {
      setSyncError(String(err));
      setSyncingDeviceId(null);
    }
  }

  return (
    <>
      <section className="rounded-xl border border-border bg-surface p-5">
        <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
          This account's devices
        </h2>
        <p className="mt-3 text-sm text-text-muted">
          Every device is fully independent — each can send and receive messages on its own.
          Pressing "Sync" reconciles message history between this device and another one, when
          both are online.
        </p>

        {loading && <p className="mt-4 text-sm text-text-faint">Loading…</p>}
        {loadError && <p className="mt-4 text-sm text-danger">{loadError}</p>}

        {!loading && !loadError && (
          <div className="mt-4 space-y-2">
            {devices.map((d) => (
              <div
                key={d.device_id}
                className="flex items-center justify-between gap-2 rounded-md border border-border bg-ink px-3 py-2"
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-sm text-text">{shortId(d.device_id)}</p>
                  <p className="mt-0.5 text-xs text-text-faint">
                    {d.is_this_device ? "This device" : d.online ? "Online" : "Offline"}
                    {lastSyncedDeviceId === d.device_id && !d.is_this_device && " · Synced"}
                  </p>
                </div>
                {!d.is_this_device && (
                  <button
                    onClick={() => handleSync(d.device_id)}
                    disabled={!d.online || syncingDeviceId === d.device_id}
                    className="shrink-0 rounded-md px-2.5 py-1.5 text-xs font-medium text-brass hover:bg-brass-wash disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {syncingDeviceId === d.device_id ? "Syncing…" : "Sync"}
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
        {syncError && <p className="mt-2 text-xs text-danger">{syncError}</p>}
      </section>

      <section className="rounded-xl border border-border bg-surface p-5">
        <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
          Add a device
        </h2>
        <p className="mt-3 text-sm text-text-muted">
          Scan this code with Seal on your phone (Settings → Add a device → Scan) to sign that
          device in as this same account.
        </p>

        {offer && qrDataUrl ? (
          <div className="mt-4 flex flex-col items-center gap-3">
            <img
              src={qrDataUrl}
              alt="Pairing QR code"
              className="rounded-lg border border-border bg-white p-3"
              width={260}
              height={260}
            />
            <p className="text-xs text-text-faint">
              {secondsLeft > 0 ? `Expires in ${secondsLeft}s` : "Expired"}
            </p>
            <div className="flex gap-2">
              <button
                onClick={async () => {
                  await navigator.clipboard.writeText(encodePairingOffer(offer));
                  setCodeCopied(true);
                }}
                className="rounded-md border border-border px-3.5 py-2 text-sm font-medium text-text-muted transition hover:border-brass-dim hover:text-brass"
              >
                {codeCopied ? "Copied" : "Copy code"}
              </button>
              <button
                onClick={handleStartPairing}
                disabled={pairingBusy}
                className="rounded-md border border-border px-3.5 py-2 text-sm font-medium text-text-muted transition hover:border-brass-dim hover:text-brass disabled:cursor-not-allowed disabled:opacity-40"
              >
                Generate a new code
              </button>
            </div>
          </div>
        ) : (
          <button
            onClick={handleStartPairing}
            disabled={pairingBusy}
            className="mt-4 rounded-md border border-border px-3.5 py-2 text-sm font-medium text-brass transition hover:-translate-y-px hover:border-brass-dim hover:bg-brass-wash active:translate-y-0 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {pairingBusy ? "Generating…" : "Show QR code"}
          </button>
        )}
        {pairingError && <p className="mt-2 text-xs text-danger">{pairingError}</p>}
      </section>
    </>
  );
}
