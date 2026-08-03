import { useEffect, useState } from "react";
import { EMBEDDED_SERVER_SENTINEL, api } from "../lib/tauri";
import type { AccountSummary, NetworkStatus } from "../lib/types";
import { CipherSeal } from "./CipherSeal";
import {
  MIC_THRESHOLD_MAX_DB,
  MIC_THRESHOLD_MIN_DB,
  getMicThresholdDb,
  saveMicThresholdDb,
} from "../lib/voiceSettings";
import {
  acceleratorFromKeyEvent,
  applyPushToTalk,
  getPttEnabled,
  getPttShortcut,
  savePttSettings,
} from "../lib/pushToTalk";
import { getAutostartEnabled, saveAutostartEnabled, syncAutostart } from "../lib/autostart";
import {
  ensureNotificationPermission,
  getNotificationsEnabled,
  saveNotificationsEnabled,
} from "../lib/notifications";

interface SettingsPanelProps {
  userId: string;
  displayName: string;
  onRename: (name: string) => Promise<void>;
  networkStatus: NetworkStatus;
  serverUrl: string | null;
  onClose: () => void;
  onPurge: () => Promise<void>;
  onOpenTutorial: () => void;
  activeAccountId: string | null;
  onSwitchAccount: (account: AccountSummary) => Promise<void>;
  onRemoveOtherAccount: (accountId: string) => Promise<void>;
  onAddAnotherAccount: () => void;
  onRemoveCurrentAccount: () => Promise<void>;
}

const CONFIRM_PHRASE = "DELETE EVERYTHING";

const NETWORK_COPY: Record<NetworkStatus, { label: string; body: string }> = {
  public: {
    label: "Directly reachable",
    body: "Peers can connect to you directly — no relay needed for a fast, low-latency path.",
  },
  private: {
    label: "Behind a NAT or firewall",
    body: "Direct dials can't reach you from outside. Seal falls back to a relay to get connected, then attempts to upgrade to a direct connection automatically.",
  },
  unknown: {
    label: "Still figuring that out",
    body: "Seal hasn't finished probing your reachability yet — this settles shortly after startup.",
  },
};

export function SettingsPanel({
  userId,
  displayName,
  onRename,
  networkStatus,
  serverUrl,
  onClose,
  onPurge,
  onOpenTutorial,
  activeAccountId,
  onSwitchAccount,
  onRemoveOtherAccount,
  onAddAnotherAccount,
  onRemoveCurrentAccount,
}: SettingsPanelProps) {
  const [confirmText, setConfirmText] = useState("");
  const [purging, setPurging] = useState(false);
  const [copied, setCopied] = useState(false);
  const [editingServer, setEditingServer] = useState(false);
  const [serverInput, setServerInput] = useState("");
  const [serverSaved, setServerSaved] = useState(false);

  const [editingName, setEditingName] = useState(false);
  const [nameInput, setNameInput] = useState("");
  const [nameSaving, setNameSaving] = useState(false);

  const [otherAccounts, setOtherAccounts] = useState<AccountSummary[]>([]);
  const [accountsLoading, setAccountsLoading] = useState(true);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [confirmRemoveCurrent, setConfirmRemoveCurrent] = useState(false);
  const [removingCurrent, setRemovingCurrent] = useState(false);

  const [micThresholdDb, setMicThresholdDb] = useState(getMicThresholdDb);
  const [pttEnabled, setPttEnabled] = useState(getPttEnabled);
  const [pttShortcut, setPttShortcut] = useState(getPttShortcut);
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [autostartEnabled, setAutostartEnabled] = useState(getAutostartEnabled);
  const [notificationsEnabled, setNotificationsEnabled] = useState(getNotificationsEnabled);

  useEffect(() => {
    if (!recordingShortcut) return;
    function handleKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      const accelerator = acceleratorFromKeyEvent(e);
      if (!accelerator) return; // only modifiers held so far — keep listening
      setPttShortcut(accelerator);
      setRecordingShortcut(false);
      savePttSettings(pttEnabled, accelerator);
      applyPushToTalk(pttEnabled, accelerator);
    }
    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", handleKeyDown, { capture: true });
  }, [recordingShortcut, pttEnabled]);

  function handleThresholdChange(db: number) {
    setMicThresholdDb(db);
    saveMicThresholdDb(db);
    api.setMicThresholdDb(db).catch(() => {});
  }

  function handleTogglePtt() {
    const next = !pttEnabled;
    setPttEnabled(next);
    savePttSettings(next, pttShortcut);
    applyPushToTalk(next, pttShortcut);
  }

  function handleToggleAutostart() {
    const next = !autostartEnabled;
    setAutostartEnabled(next);
    saveAutostartEnabled(next);
    syncAutostart(next);
  }

  function handleToggleNotifications() {
    const next = !notificationsEnabled;
    setNotificationsEnabled(next);
    saveNotificationsEnabled(next);
    if (next) ensureNotificationPermission();
  }

  useEffect(() => {
    let cancelled = false;
    api.listAccounts().then((state) => {
      if (cancelled) return;
      setOtherAccounts(state.accounts.filter((a) => a.account_id !== activeAccountId));
      setAccountsLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [activeAccountId]);

  async function handleCopy() {
    await navigator.clipboard.writeText(userId);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  async function handleSaveServer() {
    const value = serverInput.trim();
    if (!value) return;
    await api.saveServerUrlForNextLaunch(value);
    setServerSaved(true);
    setEditingServer(false);
  }

  async function handleSaveName() {
    const value = nameInput.trim();
    if (!value) return;
    setNameSaving(true);
    try {
      await onRename(value);
      setEditingName(false);
    } finally {
      setNameSaving(false);
    }
  }

  async function handleSwitch(account: AccountSummary) {
    setSwitchingId(account.account_id);
    try {
      await onSwitchAccount(account);
    } finally {
      setSwitchingId(null);
    }
  }

  async function handleRemoveOther(accountId: string) {
    await onRemoveOtherAccount(accountId);
    setOtherAccounts((prev) => prev.filter((a) => a.account_id !== accountId));
    setConfirmRemoveId(null);
  }

  async function handleRemoveCurrent() {
    setRemovingCurrent(true);
    try {
      await onRemoveCurrentAccount();
    } catch {
      setRemovingCurrent(false);
    }
  }

  async function handlePurge() {
    if (confirmText !== CONFIRM_PHRASE || purging) return;
    setPurging(true);
    try {
      await onPurge();
    } catch {
      setPurging(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex overflow-y-auto bg-ink">
      <div className="mx-auto flex w-full max-w-2xl flex-col px-8 py-10">
        <div className="mb-8 flex items-center justify-between">
          <h1 className="font-display text-2xl font-semibold text-text">Settings</h1>
          <button
            onClick={onClose}
            aria-label="Close settings"
            className="flex h-8 w-8 items-center justify-center rounded-md text-text-muted hover:scale-110 hover:bg-surface hover:text-text active:scale-90"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path d="m6 6 12 12M18 6 6 18" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </button>
        </div>

        <section className="rounded-xl border border-border bg-surface p-5">
          <div className="flex items-center gap-2">
            <CipherSeal status="secure" size={16} />
            <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
              My identity
            </h2>
          </div>

          <div className="mt-3 flex items-center justify-between gap-2">
            {editingName ? (
              <div className="flex-1">
                <input
                  autoFocus
                  value={nameInput}
                  onChange={(e) => setNameInput(e.target.value)}
                  className="w-full rounded-md border border-border bg-ink px-3 py-2 text-[15px] text-text transition-colors focus:border-brass focus:outline-none"
                />
                <div className="mt-2 flex gap-2">
                  <button
                    onClick={() => setEditingName(false)}
                    className="rounded-md px-3 py-1.5 text-xs text-text-muted hover:bg-surface-raised"
                  >
                    Cancel
                  </button>
                  <button
                    onClick={handleSaveName}
                    disabled={!nameInput.trim() || nameSaving}
                    className="rounded-md bg-brass px-3 py-1.5 text-xs font-medium text-ink disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    {nameSaving ? "Saving…" : "Save"}
                  </button>
                </div>
              </div>
            ) : (
              <>
                <p className="text-[15px] font-medium text-text">{displayName}</p>
                <button
                  onClick={() => {
                    setNameInput(displayName);
                    setEditingName(true);
                  }}
                  className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash"
                >
                  Rename
                </button>
              </>
            )}
          </div>

          <p className="mt-3 text-sm text-text-muted">
            This is your public ID — the address other people use to find and message you. It's
            derived from your private key, which never leaves this device.
          </p>
          <div className="mt-3 flex items-center gap-2 rounded-md border border-border bg-ink px-3 py-2">
            <code className="flex-1 truncate font-mono text-[13px] text-text">{userId}</code>
            <button
              onClick={handleCopy}
              className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash"
            >
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Accounts on this device
          </h2>
          <p className="mt-3 text-sm text-text-muted">
            Each account is a fully separate identity — its own keys, contacts, and messages.
          </p>

          {!accountsLoading && otherAccounts.length > 0 && (
            <div className="mt-3 space-y-2">
              {otherAccounts.map((account) => (
                <div
                  key={account.account_id}
                  className="flex items-center justify-between gap-2 rounded-md border border-border bg-ink px-3 py-2"
                >
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-text">{account.display_name}</p>
                    <p className="truncate font-mono text-xs text-text-faint">
                      {account.user_id.slice(0, 16)}…
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-1.5">
                    {confirmRemoveId === account.account_id ? (
                      <>
                        <button
                          onClick={() => setConfirmRemoveId(null)}
                          className="rounded-md px-2 py-1 text-xs text-text-muted hover:bg-surface-raised"
                        >
                          Cancel
                        </button>
                        <button
                          onClick={() => handleRemoveOther(account.account_id)}
                          className="rounded-md bg-danger px-2 py-1 text-xs font-medium text-text"
                        >
                          Confirm
                        </button>
                      </>
                    ) : (
                      <>
                        <button
                          onClick={() => handleSwitch(account)}
                          disabled={switchingId !== null}
                          className="rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash disabled:cursor-not-allowed disabled:opacity-40"
                        >
                          {switchingId === account.account_id ? "Switching…" : "Switch"}
                        </button>
                        <button
                          onClick={() => setConfirmRemoveId(account.account_id)}
                          className="rounded-md px-2 py-1 text-xs font-medium text-text-muted hover:bg-surface-raised"
                        >
                          Remove
                        </button>
                      </>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}

          <button
            onClick={onAddAnotherAccount}
            className="mt-4 rounded-md border border-border px-3.5 py-2 text-sm font-medium text-brass transition hover:-translate-y-px hover:border-brass-dim hover:bg-brass-wash active:translate-y-0"
          >
            Add another account
          </button>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Directory server
          </h2>
          <p className="mt-3 text-sm text-text-muted">
            The one server Seal talks to, for finding people by ID. Anyone can run one — see{" "}
            <code className="font-mono text-text">scripts/setup-backend.sh</code> in the project
            if you want to host your own.
          </p>
          <div className="mt-3 flex items-center gap-2 rounded-md border border-border bg-ink px-3 py-2">
            <code className="flex-1 truncate font-mono text-[13px] text-text">
              {serverUrl === EMBEDDED_SERVER_SENTINEL
                ? "Local test server (runs on this device)"
                : (serverUrl ?? "Unknown")}
            </code>
            {!editingServer && (
              <button
                onClick={() => {
                  setServerInput(serverUrl ?? "");
                  setServerSaved(false);
                  setEditingServer(true);
                }}
                className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash"
              >
                Change
              </button>
            )}
          </div>
          {editingServer && (
            <div className="mt-2.5">
              <input
                autoFocus
                value={serverInput}
                onChange={(e) => setServerInput(e.target.value)}
                placeholder="https://directory.example.com"
                className="w-full rounded-md border border-border bg-ink px-3 py-2 font-mono text-[13px] text-text transition-colors placeholder:text-text-faint focus:border-brass focus:outline-none"
              />
              <div className="mt-2 flex gap-2">
                <button
                  onClick={() => setEditingServer(false)}
                  className="rounded-md px-3 py-1.5 text-xs text-text-muted hover:bg-surface-raised"
                >
                  Cancel
                </button>
                <button
                  onClick={handleSaveServer}
                  disabled={!serverInput.trim()}
                  className="rounded-md bg-brass px-3 py-1.5 text-xs font-medium text-ink disabled:cursor-not-allowed disabled:opacity-40"
                >
                  Save
                </button>
              </div>
            </div>
          )}
          {serverSaved && (
            <p className="mt-2.5 text-xs text-brass">Saved — restart Seal for this to take effect.</p>
          )}
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            How Seal works
          </h2>
          <p className="mt-3 text-sm text-text-muted">
            A short walkthrough of how your keys, your messages, and the one server this app
            talks to actually fit together.
          </p>
          <button
            onClick={onOpenTutorial}
            className="mt-4 rounded-md border border-border px-3.5 py-2 text-sm font-medium text-brass transition hover:-translate-y-px hover:border-brass-dim hover:bg-brass-wash active:translate-y-0"
          >
            Replay the walkthrough
          </button>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Voice
          </h2>

          <div className="mt-4">
            <div className="flex items-center justify-between">
              <label className="text-sm font-medium text-text">Mic sensitivity</label>
              <span className="font-mono text-xs text-text-faint">{micThresholdDb.toFixed(0)} dB</span>
            </div>
            <p className="mt-1 text-xs text-text-muted">
              Audio quieter than this never gets sent — raise it if background noise keeps
              triggering your mic, lower it if quiet speech gets cut off.
            </p>
            <input
              type="range"
              min={MIC_THRESHOLD_MIN_DB}
              max={MIC_THRESHOLD_MAX_DB}
              step={1}
              value={micThresholdDb}
              onChange={(e) => handleThresholdChange(Number(e.target.value))}
              className="mt-2 w-full accent-verdigris"
            />
          </div>

          <div className="mt-5 border-t border-border pt-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <p className="text-sm font-medium text-text">Push to talk</p>
                <p className="mt-1 text-xs text-text-muted">
                  A system-wide keybind — held from any app, not just Seal — that unmutes your
                  mic only while it's down.
                </p>
              </div>
              <button
                onClick={handleTogglePtt}
                aria-pressed={pttEnabled}
                className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-medium transition active:scale-90 ${
                  pttEnabled ? "bg-verdigris-wash text-verdigris" : "bg-surface-raised text-text-muted"
                }`}
              >
                {pttEnabled ? "On" : "Off"}
              </button>
            </div>
            <div className="mt-3 flex items-center gap-2">
              <button
                onClick={() => setRecordingShortcut(true)}
                className="flex-1 rounded-md border border-border bg-ink px-3 py-2 text-left font-mono text-[13px] text-text hover:border-brass-dim"
              >
                {recordingShortcut ? "Press any key…" : (pttShortcut ?? "Click to set a keybind")}
              </button>
              {pttShortcut && !recordingShortcut && (
                <button
                  onClick={() => {
                    setPttShortcut(null);
                    savePttSettings(pttEnabled, null);
                    applyPushToTalk(pttEnabled, null);
                  }}
                  className="shrink-0 rounded-md px-2 py-1 text-xs text-text-muted hover:bg-surface-raised"
                >
                  Clear
                </button>
              )}
            </div>
            {pttEnabled && !pttShortcut && (
              <p className="mt-2 text-xs text-danger">Set a keybind above for push to talk to do anything.</p>
            )}
          </div>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Startup
          </h2>
          <div className="mt-4 flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-text">Launch Seal at login</p>
              <p className="mt-1 text-xs text-text-muted">
                Starts automatically when you log into this device.
              </p>
            </div>
            <button
              onClick={handleToggleAutostart}
              aria-pressed={autostartEnabled}
              className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-medium transition active:scale-90 ${
                autostartEnabled ? "bg-verdigris-wash text-verdigris" : "bg-surface-raised text-text-muted"
              }`}
            >
              {autostartEnabled ? "On" : "Off"}
            </button>
          </div>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Notifications
          </h2>
          <div className="mt-4 flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-text">Message notifications</p>
              <p className="mt-1 text-xs text-text-muted">
                A native notification and a chime for new messages while Seal is open —
                including while it's minimized to the tray.
              </p>
            </div>
            <button
              onClick={handleToggleNotifications}
              aria-pressed={notificationsEnabled}
              className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-medium transition active:scale-90 ${
                notificationsEnabled ? "bg-verdigris-wash text-verdigris" : "bg-surface-raised text-text-muted"
              }`}
            >
              {notificationsEnabled ? "On" : "Off"}
            </button>
          </div>
        </section>

        <section className="mt-6 rounded-xl border border-border bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-text-muted">
            Network
          </h2>
          <div className="mt-3 flex items-center gap-2">
            <span
              className={`h-2 w-2 rounded-full ${
                networkStatus === "public"
                  ? "bg-verdigris"
                  : networkStatus === "private"
                    ? "bg-brass"
                    : "bg-text-faint"
              }`}
            />
            <p className="text-sm font-medium text-text">{NETWORK_COPY[networkStatus].label}</p>
          </div>
          <p className="mt-2 text-sm text-text-muted">{NETWORK_COPY[networkStatus].body}</p>
        </section>

        <section className="mt-6 rounded-xl border border-danger-dim/40 bg-surface p-5">
          <h2 className="font-display text-sm font-semibold uppercase tracking-wider text-danger">
            Data &amp; privacy
          </h2>

          <div className="mt-4 flex items-center justify-between gap-3 border-b border-border pb-4">
            <div>
              <p className="text-sm font-medium text-text">Remove this account</p>
              <p className="mt-1 text-sm text-text-muted">
                Deletes {displayName}&rsquo;s keys and local history from this device only. Other
                accounts here are untouched.
              </p>
            </div>
            {confirmRemoveCurrent ? (
              <div className="flex shrink-0 gap-1.5">
                <button
                  onClick={() => setConfirmRemoveCurrent(false)}
                  className="rounded-md px-2.5 py-1.5 text-xs text-text-muted hover:bg-surface-raised"
                >
                  Cancel
                </button>
                <button
                  onClick={handleRemoveCurrent}
                  disabled={removingCurrent}
                  className="rounded-md bg-danger px-2.5 py-1.5 text-xs font-medium text-text disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {removingCurrent ? "Removing…" : "Confirm"}
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmRemoveCurrent(true)}
                className="shrink-0 rounded-md border border-danger-dim/40 px-3 py-1.5 text-xs font-medium text-danger hover:bg-danger/10"
              >
                Remove
              </button>
            )}
          </div>

          <p className="mt-4 text-sm text-text-muted">
            <strong className="text-text">Delete everything on this device.</strong> This
            permanently destroys the keys, contacts, and message history for{" "}
            <strong className="text-text">every account</strong> on this device — instantly and
            without any way back. It has no effect on your other devices or on anyone you've
            talked to; it only ever touches this copy of the app.
          </p>
          <label className="mt-4 block text-xs font-medium uppercase tracking-wider text-text-faint">
            Type <span className="font-mono text-danger">{CONFIRM_PHRASE}</span> to confirm
          </label>
          <input
            value={confirmText}
            onChange={(e) => setConfirmText(e.target.value)}
            className="mt-1.5 w-full rounded-md border border-border bg-ink px-3 py-2 font-mono text-sm text-text transition-colors focus:border-danger focus:outline-none"
            placeholder={CONFIRM_PHRASE}
          />
          <button
            onClick={handlePurge}
            disabled={confirmText !== CONFIRM_PHRASE || purging}
            className="mt-4 w-full rounded-md bg-danger py-2.5 text-sm font-medium text-text transition enabled:hover:brightness-110 enabled:active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-30"
          >
            {purging ? "Deleting…" : "Delete everything on this device"}
          </button>
        </section>
      </div>
    </div>
  );
}
