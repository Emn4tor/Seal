import { useEffect, useRef, useState, type ReactElement, type ReactNode } from "react";
import { EMBEDDED_SERVER_SENTINEL, api } from "../lib/tauri";
import { useModalEscape } from "../lib/useModalEscape";
import type { AccountSummary, NetworkStatus } from "../lib/types";
import { CipherSeal } from "./CipherSeal";
import {
  MIC_THRESHOLD_MAX_DB,
  MIC_THRESHOLD_MIN_DB,
  getMicThresholdDb,
  getPreferredInputDevice,
  getPreferredOutputDevice,
  saveMicThresholdDb,
  savePreferredInputDevice,
  savePreferredOutputDevice,
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
import { getDndEnabled, saveDndEnabled } from "../lib/dndSettings";
import { getNotificationSoundId, saveNotificationSoundId } from "../lib/notificationSoundSettings";
import { NOTIFICATION_SOUNDS, type NotificationSoundId, playMessageSound } from "../lib/notificationSound";
import { OpenModal } from "../App";
import { checkForUpdates } from "../lib/updater";
import { getVersion } from "@tauri-apps/api/app";
import { getRingtoneId, saveRingtoneId } from "../lib/ringtoneSettings";
import { RINGTONES, type RingtoneHandle, type RingtoneId, previewRingtone } from "../lib/ringtones";
import { getUpdateAutoCheckEnabled, getUpdateAutoInstallEnabled, saveUpdateAutoCheckEnabled, saveUpdateAutoInstallEnabled } from "../lib/updaterSettings";
import { ToastItem } from "./ToastStack";

interface SettingsPanelProps {
  userId: string;
  displayName: string;
  onRename: (name: string) => Promise<void>;
  networkStatus: NetworkStatus;
  /** The raw saved choice ("embedded", or a real URL) — not a resolved,
   * always-connectable URL. See `App.tsx`'s state comment for why this
   * distinction matters for the edit/re-save flow below. */
  savedServerChoice: string | null;
  onClose: () => void;
  onPurge: () => Promise<void>;
  onOpenTutorial: () => void;
  activeAccountId: string | null;
  onSwitchAccount: (account: AccountSummary) => Promise<void>;
  onRemoveOtherAccount: (accountId: string) => Promise<void>;
  onAddAnotherAccount: () => void;
  onRemoveCurrentAccount: () => Promise<void>;
  setOpenModal: React.Dispatch<React.SetStateAction<OpenModal>>;
  notify: (title: string, body: string, opts: { variant?: ToastItem["variant"]; focused?: boolean; groupId?: string }) => void;
}

const CONFIRM_PHRASE = "DELETE EVERYTHING";

const NETWORK_COPY: Record<NetworkStatus, { label: string; body: string }> = {
  public: {
    label: "Directly reachable",
    body: "Peers can connect to you directly, no relay needed for a fast, low-latency path.",
  },
  private: {
    label: "Behind a NAT or firewall",
    body: "Direct dials can't reach you from outside. Seal falls back to a relay to get connected, then attempts to upgrade to a direct connection automatically.",
  },
  unknown: {
    label: "Still figuring that out",
    body: "Seal hasn't finished probing your reachability yet, this settles shortly after startup.",
  },
};

type Tab = "account" | "notifications" | "voice" | "advanced" | "privacy";

const TABS: { id: Tab; label: string; icon: (props: { className?: string }) => ReactElement }[] = [
  { id: "account", label: "Account", icon: AccountIcon },
  { id: "notifications", label: "Notifications", icon: BellIcon },
  { id: "voice", label: "Voice & Audio", icon: MicIcon },
  { id: "advanced", label: "Advanced", icon: SlidersIcon },
  { id: "privacy", label: "Privacy & Data", icon: ShieldIcon },
];

function AccountIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" className={className}>
      <circle cx="12" cy="8" r="3.4" stroke="currentColor" strokeWidth="1.5" />
      <path d="M5 20c1.2-4 4-6 7-6s5.8 2 7 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function BellIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M6 10a6 6 0 1 1 12 0c0 4 1.5 5.5 1.5 5.5H4.5S6 14 6 10Z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path d="M10 18.5a2 2 0 0 0 4 0" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function MicIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" className={className}>
      <rect x="9" y="3" width="6" height="11" rx="3" stroke="currentColor" strokeWidth="1.5" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function SlidersIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" className={className}>
      <path d="M4 6h10M17 6h3M4 12h3M9 12h11M4 18h13M20 18h0" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="14" cy="6" r="2" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="6" cy="12" r="2" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="16" cy="18" r="2" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}

function ShieldIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M12 3.5 5 6v6c0 4.5 3 7.5 7 8.5 4-1 7-4 7-8.5V6l-7-2.5Z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function UpdateIcon({ className }: { className?: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" className={className}>
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M3 21v-5h5" />
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
    </svg>
  );
}

function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-pressed={on}
      className={`shrink-0 rounded-full px-3 py-1.5 text-xs font-medium transition active:scale-90 ${
        on ? "bg-verdigris-wash text-verdigris" : "bg-surface-raised text-text-muted"
      }`}
    >
      {on ? "On" : "Off"}
    </button>
  );
}

function Section({ title, danger, children }: { title: string; danger?: boolean; children: ReactNode }) {
  return (
    <section className={`rounded-xl border p-5 ${danger ? "border-danger-dim/40 bg-surface" : "border-border bg-surface"}`}>
      <h2
        className={`font-display text-sm font-semibold uppercase tracking-wider ${danger ? "text-danger" : "text-text-muted"}`}
      >
        {title}
      </h2>
      {children}
    </section>
  );
}

export function SettingsPanel({
  userId,
  displayName,
  onRename,
  networkStatus,
  savedServerChoice,
  onClose,
  onPurge,
  onOpenTutorial,
  activeAccountId,
  onSwitchAccount,
  onRemoveOtherAccount,
  onAddAnotherAccount,
  onRemoveCurrentAccount,
  setOpenModal,
  notify
}: SettingsPanelProps) {
  const [tab, setTab] = useState<Tab>("account");

  const [confirmText, setConfirmText] = useState("");
  const [purging, setPurging] = useState(false);
  const [purgeError, setPurgeError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [editingServer, setEditingServer] = useState(false);
  const [serverInput, setServerInput] = useState("");
  const [serverSaved, setServerSaved] = useState(false);
  const [serverSaveError, setServerSaveError] = useState<string | null>(null);

  const [editingName, setEditingName] = useState(false);
  const [nameInput, setNameInput] = useState("");
  const [nameSaving, setNameSaving] = useState(false);

  const [otherAccounts, setOtherAccounts] = useState<AccountSummary[]>([]);
  const [accountsLoading, setAccountsLoading] = useState(true);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [confirmRemoveCurrent, setConfirmRemoveCurrent] = useState(false);
  const [removingCurrent, setRemovingCurrent] = useState(false);
  const [removeCurrentError, setRemoveCurrentError] = useState<string | null>(null);
  const [removingOtherId, setRemovingOtherId] = useState<string | null>(null);
  const [removeOtherError, setRemoveOtherError] = useState<string | null>(null);

  const [micThresholdDb, setMicThresholdDb] = useState(getMicThresholdDb);
  const [pttEnabled, setPttEnabled] = useState(getPttEnabled);
  const [pttShortcut, setPttShortcut] = useState(getPttShortcut);
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [autostartEnabled, setAutostartEnabled] = useState(getAutostartEnabled);
  const [notificationsEnabled, setNotificationsEnabled] = useState(getNotificationsEnabled);
  const [dndEnabled, setDndEnabled] = useState(getDndEnabled);
  const [notificationSoundId, setNotificationSoundId] = useState(getNotificationSoundId);
  const [ringtoneId, setRingtoneId] = useState(getRingtoneId);
  const ringtonePreviewRef = useRef<RingtoneHandle | null>(null);

  const [inputDevices, setInputDevices] = useState<string[]>([]);
  const [outputDevices, setOutputDevices] = useState<string[]>([]);
  const [preferredInput, setPreferredInput] = useState(getPreferredInputDevice);
  const [preferredOutput, setPreferredOutput] = useState(getPreferredOutputDevice);
  const [devicesError, setDevicesError] = useState<string | null>(null);

  const [autoCheckUpdates, setAutoCheckUpdates] = useState(getUpdateAutoCheckEnabled);
  const [autoInstallUpdates, setAutoInstallUpdates] = useState(getUpdateAutoInstallEnabled);

  const [appVersion, setAppVersion] = useState<string>("unknown");

  useModalEscape(onClose);

  useEffect(() => {
    async function fetchVersion() {
      try {
        const version = await getVersion();
        setAppVersion(version);
      } catch (error) {
        notify("Version Checking Failed", "Error while getting app version.", {variant:"error"});
      }
    }

    fetchVersion();
  }, []);

  useEffect(() => {
    if (tab !== "voice") return;
    setDevicesError(null);
    Promise.all([api.listInputDevices(), api.listOutputDevices()])
      .then(([inputs, outputs]) => {
        setInputDevices(inputs);
        setOutputDevices(outputs);
      })
      .catch((err) => setDevicesError(String(err)));
  }, [tab]);

  useEffect(() => {
    if (!recordingShortcut) return;
    function handleKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      // Without this, the same keydown also reaches `useModalEscape`'s
      // bubble-phase listener on `document`. Pressing Escape to cancel
      // the recording used to also close the whole Settings panel out
      // from under it.
      e.stopPropagation();
      if (e.code === "Escape") {
        setRecordingShortcut(false);
        return;
      }
      const accelerator = acceleratorFromKeyEvent(e);
      if (!accelerator) return; // only modifiers held so far, keep listening
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

  function handleToggleUpdateAutoCheck() {
    const next = !autoCheckUpdates;
    setAutoCheckUpdates(next);
    saveUpdateAutoCheckEnabled(next);
    if (!next) {
      setAutoInstallUpdates(false);
      saveUpdateAutoInstallEnabled(false);
    }
  }
  function handleToggleUpdateAutoInstall() {
    const next = !autoInstallUpdates;
    if (autoCheckUpdates) {
      setAutoInstallUpdates(next);
      saveUpdateAutoInstallEnabled(next);
    }
  }

  function handleToggleNotifications() {
    const next = !notificationsEnabled;
    setNotificationsEnabled(next);
    saveNotificationsEnabled(next);
    if (next) ensureNotificationPermission();
  }

  function handleToggleDnd() {
    const next = !dndEnabled;
    setDndEnabled(next);
    saveDndEnabled(next);
  }

  function handlePickNotificationSound(id: NotificationSoundId) {
    setNotificationSoundId(id);
    saveNotificationSoundId(id);
    playMessageSound(id);
  }

  function handlePickRingtone(id: RingtoneId) {
    setRingtoneId(id);
    saveRingtoneId(id);
    ringtonePreviewRef.current?.stop();
    ringtonePreviewRef.current = previewRingtone(id);
  }

  useEffect(() => {
    return () => ringtonePreviewRef.current?.stop();
  }, []);

  function handlePickInputDevice(name: string) {
    const value = name || null;
    setPreferredInput(value);
    savePreferredInputDevice(value);
  }

  function handlePickOutputDevice(name: string) {
    const value = name || null;
    setPreferredOutput(value);
    savePreferredOutputDevice(value);
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
    setServerSaveError(null);
    try {
      await api.saveServerUrlForNextLaunch(value);
      setServerSaved(true);
      setEditingServer(false);
    } catch (err) {
      setServerSaveError(String(err));
    }
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
    setRemovingOtherId(accountId);
    setRemoveOtherError(null);
    try {
      await onRemoveOtherAccount(accountId);
      setOtherAccounts((prev) => prev.filter((a) => a.account_id !== accountId));
      setConfirmRemoveId(null);
    } catch (err) {
      setRemoveOtherError(String(err));
    } finally {
      setRemovingOtherId(null);
    }
  }

  async function handleRemoveCurrent() {
    setRemovingCurrent(true);
    setRemoveCurrentError(null);
    try {
      await onRemoveCurrentAccount();
    } catch (err) {
      setRemovingCurrent(false);
      setRemoveCurrentError(String(err));
    }
  }

  async function handlePurge() {
    if (confirmText !== CONFIRM_PHRASE || purging) return;
    setPurging(true);
    setPurgeError(null);
    try {
      await onPurge();
    } catch (err) {
      setPurging(false);
      setPurgeError(String(err));
    }
  }

  async function checkUpdates() {
    try {
      let updateAvailable = await checkForUpdates();
      if (updateAvailable) {
        setOpenModal("update-available");
      } else {
        setOpenModal("up-to-date");
      }
    } catch (e) {
      notify("Update Checking Failed", "Update endpoint did not respond with a successful status code.", {variant:"error"})
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex bg-ink">
      <div className="flex w-full">
        <div className="flex w-56 shrink-0 flex-col border-r border-border bg-surface/40 px-3 py-6">
          <h1 className="mb-6 px-2 font-display text-xl font-semibold text-text">Settings</h1>
          <nav className="flex flex-col gap-0.5">
            {TABS.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                aria-current={tab === id}
                className={`flex items-center gap-2.5 rounded-md px-3 py-2 text-left text-sm font-medium transition-all ${
                  tab === id ? "bg-brass-wash text-brass" : "text-text-muted hover:bg-surface-raised hover:text-text"
                }`}
              >
                <Icon className="shrink-0" />
                {label}
              </button>
            ))}
            <button
              className="flex items-center gap-2.5 rounded-md px-3 py-2 text-left text-sm font-medium transition-all text-text-muted hover:bg-green-600/10 hover:text-text active:scale-98"
              onClick={() => checkUpdates()}
            >
              <UpdateIcon className="shrink-0" />
              Check for Updates
            </button>
          </nav>
          <p className="flex-1"></p>
          <p className="text-xs px-2 text-text-muted">version: {appVersion}</p>
        </div>

        <div className="flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-2xl flex-col gap-6 px-8 py-10">
            <div className="flex items-center justify-end">
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

            {tab === "account" && (
              <>
                <Section title="My identity">
                  <div className="mt-3 flex items-center gap-2">
                    <CipherSeal status="secure" size={16} />
                    <div className="flex-1">
                      {editingName ? (
                        <div>
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
                        <div className="flex items-center justify-between gap-2">
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
                        </div>
                      )}
                    </div>
                  </div>

                  <p className="mt-3 text-sm text-text-muted">
                    This is your public ID, the address other people use to find and message you. It's
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
                </Section>

                <Section title="Accounts on this device">
                  <p className="mt-3 text-sm text-text-muted">
                    Each account is a fully separate identity, its own keys, contacts, and messages.
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
                                  disabled={removingOtherId === account.account_id}
                                  className="rounded-md px-2 py-1 text-xs text-text-muted hover:bg-surface-raised disabled:cursor-not-allowed disabled:opacity-40"
                                >
                                  Cancel
                                </button>
                                <button
                                  onClick={() => handleRemoveOther(account.account_id)}
                                  disabled={removingOtherId === account.account_id}
                                  className="rounded-md bg-danger px-2 py-1 text-xs font-medium text-text disabled:cursor-not-allowed disabled:opacity-60"
                                >
                                  {removingOtherId === account.account_id ? "Removing…" : "Confirm"}
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
                  {removeOtherError && <p className="mt-2 text-xs text-danger">{removeOtherError}</p>}

                  <button
                    onClick={onAddAnotherAccount}
                    className="mt-4 rounded-md border border-border px-3.5 py-2 text-sm font-medium text-brass transition hover:-translate-y-px hover:border-brass-dim hover:bg-brass-wash active:translate-y-0"
                  >
                    Add another account
                  </button>
                </Section>
              </>
            )}

            {tab === "notifications" && (
              <>
                <Section title="Notifications">
                  <div className="mt-4 flex items-center justify-between gap-3">
                    <div>
                      <p className="text-sm font-medium text-text">Message notifications</p>
                      <p className="mt-1 text-xs text-text-muted">
                        A native notification and a chime for new messages while Seal is open -
                        including while it's minimized to the tray.
                      </p>
                    </div>
                    <Toggle on={notificationsEnabled} onClick={handleToggleNotifications} />
                  </div>

                  <div className="mt-5 flex items-center justify-between gap-3 border-t border-border pt-4">
                    <div>
                      <p className="text-sm font-medium text-text">Do Not Disturb</p>
                      <p className="mt-1 text-xs text-text-muted">
                        Mutes every message sound. Native popups and in-app banners still show
                        normally, this only silences the chime.
                      </p>
                    </div>
                    <Toggle on={dndEnabled} onClick={handleToggleDnd} />
                  </div>

                  <p className="mt-5 text-xs text-text-muted">
                    A message in the conversation you're currently looking at always plays a
                    sound, but never shows a popup, no point interrupting you about something
                    already on screen.
                  </p>
                </Section>

                <Section title="Notification sound">
                  <p className="mt-3 text-sm text-text-muted">Pick a chime, or preview one before choosing.</p>
                  <div className="mt-3 space-y-2">
                    {(Object.keys(NOTIFICATION_SOUNDS) as NotificationSoundId[]).map((id) => (
                      <button
                        key={id}
                        onClick={() => handlePickNotificationSound(id)}
                        className={`flex w-full items-center justify-between gap-3 rounded-md border px-3 py-2.5 text-left transition ${
                          notificationSoundId === id
                            ? "border-brass-dim bg-brass-wash"
                            : "border-border bg-ink hover:border-brass-dim/60"
                        }`}
                      >
                        <div className="min-w-0">
                          <p className={`text-sm font-medium ${notificationSoundId === id ? "text-brass" : "text-text"}`}>
                            {NOTIFICATION_SOUNDS[id].label}
                          </p>
                          <p className="mt-0.5 text-xs text-text-muted">{NOTIFICATION_SOUNDS[id].description}</p>
                        </div>
                        <span
                          onClick={(e) => {
                            e.stopPropagation();
                            playMessageSound(id);
                          }}
                          role="button"
                          aria-label={`Preview ${NOTIFICATION_SOUNDS[id].label}`}
                          className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash"
                        >
                          Preview
                        </span>
                      </button>
                    ))}
                  </div>
                </Section>

                <Section title="Ringtone">
                  <p className="mt-3 text-sm text-text-muted">
                    Plays on a loop while a 1:1 call is ringing. Picking one previews it briefly.
                  </p>
                  <div className="mt-3 space-y-2">
                    {(Object.keys(RINGTONES) as RingtoneId[]).map((id) => (
                      <button
                        key={id}
                        onClick={() => handlePickRingtone(id)}
                        className={`flex w-full items-center justify-between gap-3 rounded-md border px-3 py-2.5 text-left transition ${
                          ringtoneId === id
                            ? "border-brass-dim bg-brass-wash"
                            : "border-border bg-ink hover:border-brass-dim/60"
                        }`}
                      >
                        <div className="min-w-0">
                          <p className={`text-sm font-medium ${ringtoneId === id ? "text-brass" : "text-text"}`}>
                            {RINGTONES[id].label}
                          </p>
                          <p className="mt-0.5 text-xs text-text-muted">{RINGTONES[id].description}</p>
                        </div>
                        <span
                          onClick={(e) => {
                            e.stopPropagation();
                            ringtonePreviewRef.current?.stop();
                            ringtonePreviewRef.current = previewRingtone(id);
                          }}
                          role="button"
                          aria-label={`Preview ${RINGTONES[id].label}`}
                          className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash"
                        >
                          Preview
                        </span>
                      </button>
                    ))}
                  </div>
                </Section>

                <Section title="Muted groups">
                  <p className="mt-3 text-sm text-text-muted">
                    Mute a specific group from its member list (the bell icon next to its name),
                    for a set amount of time or indefinitely, it stays out of your notifications
                    without affecting anyone else in it.
                  </p>
                </Section>
              </>
            )}

            {tab === "voice" && (
              <>
                <Section title="Microphone">
                  <div className="mt-4">
                    <div className="flex items-center justify-between">
                      <label className="text-sm font-medium text-text">Sensitivity</label>
                      <span className="font-mono text-xs text-text-faint">{micThresholdDb.toFixed(0)} dB</span>
                    </div>
                    <p className="mt-1 text-xs text-text-muted">
                      Audio quieter than this never gets sent, raise it if background noise keeps
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
                    <label className="text-sm font-medium text-text">Input device</label>
                    <p className="mt-1 text-xs text-text-muted">
                      Takes effect the next time you join a voice channel, switching mid-call
                      isn't supported.
                    </p>
                    <select
                      value={preferredInput ?? ""}
                      onChange={(e) => handlePickInputDevice(e.target.value)}
                      className="mt-2 w-full rounded-md border border-border bg-ink px-3 py-2 text-sm text-text focus:border-brass-dim focus:outline-none"
                    >
                      <option value="">System default</option>
                      {inputDevices.map((name) => (
                        <option key={name} value={name}>
                          {name}
                        </option>
                      ))}
                    </select>
                  </div>

                  <div className="mt-5 border-t border-border pt-4">
                    <label className="text-sm font-medium text-text">Push to talk</label>
                    <div className="mt-2 flex items-center justify-between gap-3">
                      <p className="text-xs text-text-muted">
                        A system-wide keybind held from any app, that unmutes
                        your mic only while it's down.
                      </p>
                      <Toggle on={pttEnabled} onClick={handleTogglePtt} />
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
                </Section>

                <Section title="Speaker">
                  <div className="mt-4">
                    <label className="text-sm font-medium text-text">Output device</label>
                    <p className="mt-1 text-xs text-text-muted">Same as the input device: applies on your next join.</p>
                    <select
                      value={preferredOutput ?? ""}
                      onChange={(e) => handlePickOutputDevice(e.target.value)}
                      className="mt-2 w-full rounded-md border border-border bg-ink px-3 py-2 text-sm text-text focus:border-brass-dim focus:outline-none"
                    >
                      <option value="">System default</option>
                      {outputDevices.map((name) => (
                        <option key={name} value={name}>
                          {name}
                        </option>
                      ))}
                    </select>
                  </div>
                  {devicesError && <p className="mt-3 text-xs text-danger">{devicesError}</p>}
                </Section>
              </>
            )}

            {tab === "advanced" && (
              <>
                <Section title="Directory server">
                  <p className="mt-3 text-sm text-text-muted">
                    The one server Seal talks to, for finding people by ID. Anyone can run one: see{" "}
                    <code className="font-mono text-text">scripts/setup-backend.sh</code> in the project
                    if you want to host your own.
                  </p>
                  <div className="mt-3 flex items-center gap-2 rounded-md border border-border bg-ink px-3 py-2">
                    <code className="flex-1 truncate font-mono text-[13px] text-text">
                      {savedServerChoice === EMBEDDED_SERVER_SENTINEL
                        ? "Local test server (runs on this device)"
                        : (savedServerChoice ?? "Unknown")}
                    </code>
                    {!editingServer && (
                      <button
                        onClick={() => {
                          setServerInput(
                            savedServerChoice === EMBEDDED_SERVER_SENTINEL ? "" : (savedServerChoice ?? ""),
                          );
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
                      {serverSaveError && <p className="mt-2 text-xs text-danger">{serverSaveError}</p>}
                    </div>
                  )}
                  {serverSaved && (
                    <p className="mt-2.5 text-xs text-brass">Saved, restart Seal for this to take effect.</p>
                  )}
                </Section>

                <Section title="Startup">
                  <div className="mt-4 flex items-center justify-between gap-3">
                    <div>
                      <p className="text-sm font-medium text-text">Launch Seal at login</p>
                      <p className="mt-1 text-xs text-text-muted">
                        Starts automatically when you log into this device.
                      </p>
                    </div>
                    <Toggle on={autostartEnabled} onClick={handleToggleAutostart} />
                  </div>
                </Section>

                <Section title="Updates">
                  <div className="mt-4 flex items-center justify-between gap-3">
                    <div>
                      <p className="text-sm font-medium text-text">Check For Updates on Startup</p>
                      <p className="mt-1 text-xs text-text-muted">
                        Automatically check for updates on startup.
                      </p>
                    </div>
                    <Toggle on={autoCheckUpdates} onClick={handleToggleUpdateAutoCheck} />
                  </div>
                  <div className={"mt-4 flex items-center justify-between gap-3" + (autoCheckUpdates ? "" : " opacity-40") }>
                    <div>
                      <p className="text-sm font-medium text-text">Update on Startup</p>
                      <p className="mt-1 text-xs text-text-muted">
                        Automatically download and install updates on startup.
                      </p>
                    </div>
                    <Toggle on={autoInstallUpdates} onClick={handleToggleUpdateAutoInstall} />
                  </div>
                </Section>

                <Section title="Network">
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
                </Section>

                <Section title="How Seal works">
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
                </Section>
              </>
            )}

            {tab === "privacy" && (
              <Section title="Data & privacy" danger>
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
                {removeCurrentError && <p className="mt-2 text-xs text-danger">{removeCurrentError}</p>}

                <p className="mt-4 text-sm text-text-muted">
                  <strong className="text-text">Delete everything on this device.</strong> This
                  permanently destroys the keys, contacts, and message history for{" "}
                  <strong className="text-text">every account</strong> on this device — instantly and
                  without any way back. It has no effect on your other devices or on anyone you've
                  talked to; it only ever touches this copy of the app. There's no export or backup
                  feature yet, so there's nothing to save first if you want to keep any of this.
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
                {purgeError && <p className="mt-2 text-xs text-danger">{purgeError}</p>}
              </Section>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

