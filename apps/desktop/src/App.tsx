import { useEffect, useRef, useState } from "react";
import { api, onChatEvent, onContactsUpdated, onGroupsUpdated } from "./lib/tauri";
import { useChatStore } from "./store/useChatStore";
import { Onboarding } from "./components/Onboarding";
import { AccountPicker } from "./components/AccountPicker";
import { ServerChoice } from "./components/ServerChoice";
import { Sidebar } from "./components/Sidebar";
import { ConversationList } from "./components/ConversationList";
import { ChatPane, EmptyChatPane } from "./components/ChatPane";
import { MemberList } from "./components/MemberList";
import { Modal } from "./components/Modal";
import { CreateChannelModal } from "./components/CreateChannelModal";
import { SettingsPanel } from "./components/SettingsPanel";
import { Splash } from "./components/Splash";
import { TutorialWizard } from "./components/TutorialWizard";
import { CipherSeal, type SealStatus } from "./components/CipherSeal";
import { VoiceCallPanel } from "./components/VoiceCallPanel";
import { CallOverlay, type ActiveCallInfo } from "./components/CallOverlay";
import { ToastStack, type ToastItem } from "./components/ToastStack";
import { applyPushToTalk, getPttEnabled, getPttShortcut } from "./lib/pushToTalk";
import { getAutostartEnabled, syncAutostart } from "./lib/autostart";
import { ensureNotificationPermission, playNotificationSound, showNotificationPopup } from "./lib/notifications";
import { isGroupMuted } from "./lib/mutedConversations";
import { getPreferredInputDevice, getPreferredOutputDevice } from "./lib/voiceSettings";
import { playVoiceJoinSound, playVoiceLeaveSound } from "./lib/voiceSounds";
import { getRingtoneId } from "./lib/ringtoneSettings";
import { type RingtoneHandle, startCallingSound, startRingtone } from "./lib/ringtones";
import type { AccountSummary, ChannelKind } from "./lib/types";
import { checkForUpdates, update } from "./lib/updater";
import { getUpdateAutoCheckEnabled, getUpdateAutoInstallEnabled } from "./lib/updaterSettings";

const TOAST_LIFETIME_MS = 6000;

export type OpenModal = "add-contact" | "create-group" | "invite" | "create-channel" | "update-available" | "up-to-date" | null;
export type Phase = "loading" | "boot-error" | "choose-server" | "onboarding" | "picker" | "ready";
const TUTORIAL_SEEN_KEY = "seal-tutorial-seen";

/** Whether `conversationId` is the one currently open on screen — the same
 * rule `useChatStore`'s own `appendMessage` uses to decide whether to bump
 * the unread count, reused here to decide whether an incoming message is
 * worth a desktop notification (no point notifying about the conversation
 * you're already looking at). */
function isConversationOpen(conversationId: string): boolean {
  const selected = useChatStore.getState().selected;
  const selectedId =
    selected?.kind === "dm"
      ? selected.userId
      : selected?.kind === "group"
        ? `${selected.groupId}:${selected.channelId}`
        : null;
  return selectedId === conversationId;
}

export default function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [splashDone, setSplashDone] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const [serverUrl, setServerUrl] = useState<string | null>(null);
  // The raw choice as saved/passed to `startBackend` — e.g. the literal
  // "embedded" sentinel — as opposed to `serverUrl`, which is always the
  // *resolved* connectable URL (`start_backend` turns "embedded" into a
  // real `http://127.0.0.1:47100`-style address). Settings' "change
  // server" flow needs this raw form: pre-filling and re-saving the
  // resolved URL instead of the sentinel silently converts "use the local
  // embedded server" into a hardcoded address nothing will be listening on
  // the next time the embedded server picks a fresh port/needs restarting.
  const [savedServerChoice, setSavedServerChoice] = useState<string | null>(null);
  const [activeAccountId, setActiveAccountId] = useState<string | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [onboardingMode, setOnboardingMode] = useState<"first" | "add">("first");
  const [onboardingReturnPhase, setOnboardingReturnPhase] = useState<Phase>("picker");
  const [pickerBusy, setPickerBusy] = useState(false);
  const [pickerError, setPickerError] = useState<string | null>(null);
  const [openModal, setOpenModal] = useState<OpenModal>(null);
  const [createChannelKind, setCreateChannelKind] = useState<ChannelKind>("text");
  const [showSettings, setShowSettings] = useState(false);
  const [showTutorial, setShowTutorial] = useState(false);
  const [messagesError, setMessagesError] = useState<string | null>(null);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [activeCall, setActiveCall] = useState<ActiveCallInfo | null>(null);
  const ringtoneRef = useRef<RingtoneHandle | null>(null);
  // Mirrors `activeCall` for the `onChatEvent` handler below, which is
  // registered once per `userId` (see its effect's dependency array) and so
  // otherwise closes over a stale value rather than whatever's current by
  // the time a `call_ended` event actually arrives.
  const activeCallRef = useRef<ActiveCallInfo | null>(null);
  useEffect(() => {
    activeCallRef.current = activeCall;
  }, [activeCall]);
  // Bumped on every `handleStartCall`/`handleEndCall` — lets an in-flight
  // `callContact()` (the directory lookup + dial, which can take a while
  // and can fail, e.g. the peer's last-known address is stale because
  // they're not actually online) recognize it's been superseded — cancelled
  // locally, or overtaken by a fresh call — by the time it settles, so it
  // knows to clean up the real `pending_call` the backend ends up with
  // instead of leaving it stuck there forever (see `handleStartCall`).
  const callAttemptRef = useRef(0);

  function dismissToast(id: string) {
    setToasts((ts) => ts.filter((t) => t.id !== id));
  }

  function stopRingtone() {
    ringtoneRef.current?.stop();
    ringtoneRef.current = null;
  }

  // The sound always plays (a message landing while you're looking right at
  // that conversation should still be audible) unless Do Not Disturb is on
  // or the message's group is muted; the *popup* (native banner + in-app
  // toast — the same "you weren't looking, here's what happened" concept,
  // see `ToastStack.tsx`) only shows when the conversation isn't the one
  // currently open, since showing it for something already on screen is
  // just noise.
  function notify(
    title: string,
    body: string,
    opts: { variant?: ToastItem["variant"]; focused?: boolean; groupId?: string } = {},
  ) {
    const { variant = "message", focused = false, groupId } = opts;
    if (groupId && isGroupMuted(groupId)) return;

    playNotificationSound();
    if (focused) return;

    void showNotificationPopup(title, body);
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setToasts((ts) => [...ts, { id, title, body, variant }]);
    setTimeout(() => dismissToast(id), TOAST_LIFETIME_MS);
  }

  // Registered once for the app's whole lifetime, independent of login/boot
  // state — a real OS-level global shortcut (works no matter which app has
  // focus), not a page listener, so it has to live here rather than inside
  // whichever screen happens to be showing.
  useEffect(() => {
    applyPushToTalk(getPttEnabled(), getPttShortcut());
    syncAutostart(getAutostartEnabled());
    ensureNotificationPermission();
  }, []);

  const {
    userId,
    setUserId,
    displayName,
    setDisplayName,
    contacts,
    setContacts,
    groups,
    setGroups,
    upsertGroup,
    messagesByConversation,
    setMessages,
    appendMessage,
    selected,
    select,
    unread,
    networkStatus,
    setNetworkStatus,
    voicePresence,
    setChannelVoicePresence,
    reset: resetChatStore,
  } = useChatStore();

  async function refreshContacts() {
    setContacts(await api.listContacts());
  }
  async function refreshGroups() {
    setGroups(await api.listGroups());
  }

  async function checkUpdates() {
    try {
      let updateAvailable = await checkForUpdates();
      if (updateAvailable) {
        if (getUpdateAutoInstallEnabled()) {
          notify("Update", "Downloading and installing updates.")
          await update();
          return;
        } else {
          setOpenModal("update-available");
        }
      }
    } catch (e) {
      notify("Update Checking Failed", "Update endpoint did not respond with a successful status code.", {variant:"error"})
    }
  }

  useEffect(() => {
    if (getUpdateAutoCheckEnabled()) {
      checkUpdates()
    }
  }, [])

  async function finishBootWithAccount(account: AccountSummary, isNewAccount: boolean) {
    setActiveAccountId(account.account_id);
    setUserId(account.user_id);
    setDisplayName(account.display_name);
    await Promise.all([refreshContacts(), refreshGroups()]);
    setPhase("ready");
    if (isNewAccount && localStorage.getItem(TUTORIAL_SEEN_KEY) !== "1") {
      setShowTutorial(true);
    }
  }

  async function bootAccounts(resolvedServerUrl: string) {
    const decision = await api.resolveBootAccount();
    switch (decision.action) {
      case "resume":
        await finishBootWithAccount(
          await api.resumeAccount(resolvedServerUrl, decision.account.account_id),
          false,
        );
        break;
      case "createWithName":
        await finishBootWithAccount(
          await api.createAccount(resolvedServerUrl, decision.display_name),
          true,
        );
        break;
      case "needsFirstAccount":
        setOnboardingMode("first");
        setPhase("onboarding");
        break;
      case "needsPicker":
        setAccounts(decision.accounts);
        setPhase("picker");
        break;
    }
  }

  async function handleServerChosen(url: string) {
    const resolved = await api.startBackend(url);
    setServerUrl(resolved);
    setSavedServerChoice(url);
    await bootAccounts(resolved);
  }

  // Retries because `AppPaths` (and so `get_saved_server_url`) becomes
  // available essentially immediately after startup, but not provably
  // before this effect's first tick — rather than assume the exact
  // ordering, poll briefly the same way the rest of this bootstrap does.
  //
  // Everything past that retry loop is wrapped in a try/catch: this used to
  // let any failure here (a stale connection after the window was
  // backgrounded/asleep a while, a directory server that's briefly
  // unreachable, etc.) leave `phase` stuck at "loading" forever with no
  // error and no way to recover short of force-quitting. Now a failure
  // lands on a real screen with a retry button instead.
  async function bootFromSavedServer(isCancelled: () => boolean) {
    let saved: string | null = null;
    for (let attempt = 0; attempt < 200 && !isCancelled(); attempt++) {
      try {
        saved = await api.getSavedServerUrl();
        break;
      } catch {
        await new Promise((r) => setTimeout(r, 100));
      }
    }
    if (isCancelled()) return;

    if (!saved) {
      setPhase("choose-server");
      return;
    }

    try {
      const resolved = await api.startBackend(saved);
      if (isCancelled()) return;
      setServerUrl(resolved);
      setSavedServerChoice(saved);
      await bootAccounts(resolved);
    } catch (err) {
      if (isCancelled()) return;
      setBootError(String(err));
      setPhase("boot-error");
    }
  }

  useEffect(() => {
    let cancelled = false;
    bootFromSavedServer(() => cancelled);
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function handleRetryBoot() {
    setBootError(null);
    setPhase("loading");
    bootFromSavedServer(() => false);
  }

  async function handleCreateAccount(name: string) {
    if (!serverUrl) return;
    await finishBootWithAccount(await api.createAccount(serverUrl, name), true);
  }

  function startAddAccount(returnTo: Phase) {
    setShowSettings(false);
    setOnboardingReturnPhase(returnTo);
    setOnboardingMode("add");
    setPhase("onboarding");
  }

  async function handleChooseAccount(account: AccountSummary) {
    if (!serverUrl) return;
    setPickerBusy(true);
    setPickerError(null);
    try {
      await finishBootWithAccount(await api.resumeAccount(serverUrl, account.account_id), false);
    } catch (err) {
      setPickerError(String(err));
      setPickerBusy(false);
    }
  }

  async function handleSwitchAccount(account: AccountSummary) {
    if (!serverUrl) return;
    resetChatStore();
    await finishBootWithAccount(await api.resumeAccount(serverUrl, account.account_id), false);
    setShowSettings(false);
  }

  async function afterAccountRemoved() {
    resetChatStore();
    setActiveAccountId(null);
    const state = await api.listAccounts();
    if (state.accounts.length === 0) {
      setOnboardingMode("first");
      setPhase("onboarding");
    } else {
      setAccounts(state.accounts);
      setPhase("picker");
    }
  }

  async function handleRemoveOtherAccount(accountId: string) {
    await api.removeAccount(accountId);
  }

  async function handleRemoveCurrentAccount() {
    if (!activeAccountId) return;
    await api.removeAccount(activeAccountId);
    setShowSettings(false);
    await afterAccountRemoved();
  }

  useEffect(() => {
    if (!userId) return;
    const unlistenPromises = [
      onChatEvent((event) => {
        if (event.type === "direct_message") {
          const wasOpen = isConversationOpen(event.from);
          appendMessage(event.from, {
            sender_user_id: event.from,
            body: event.body,
            attachment: event.attachment,
            sent_at: Date.now() / 1000,
          });
          const sender = useChatStore.getState().contacts.find((c) => c.user_id === event.from);
          notify(
            sender?.display_name ?? event.from.slice(0, 12),
            event.body || (event.attachment ? "Sent an attachment" : ""),
            { focused: wasOpen },
          );
        } else if (event.type === "group_message") {
          const conversationId = `${event.group_id}:${event.channel_id}`;
          const wasOpen = isConversationOpen(conversationId);
          appendMessage(conversationId, {
            sender_user_id: event.from,
            body: event.body,
            attachment: event.attachment,
            sent_at: Date.now() / 1000,
          });
          const state = useChatStore.getState();
          const group = state.groups.find((g) => g.group_id === event.group_id);
          const channel = group?.channels.find((c) => c.channel_id === event.channel_id);
          const sender = state.contacts.find((c) => c.user_id === event.from);
          const senderName = sender?.display_name ?? event.from.slice(0, 12);
          const bodyText = event.body || (event.attachment ? "Sent an attachment" : "");
          notify(
            group ? (channel ? `${group.name} · #${channel.name}` : group.name) : "Group message",
            `${senderName}: ${bodyText}`,
            { focused: wasOpen, groupId: event.group_id },
          );
        } else if (event.type === "network_status") {
          setNetworkStatus(event.status);
        } else if (event.type === "voice_participants_changed") {
          // Kept up to date regardless of whether we're actually in this
          // channel's call — see `channel_voice_participants` on the Rust
          // side — so the channel list can preview who's in it before
          // joining, not just show a live roster once already in.
          setChannelVoicePresence(event.channel_id, event.user_ids);
        } else if (event.type === "message_send_failed") {
          const sender = useChatStore
            .getState()
            .contacts.find((c) => c.user_id === event.peer_user_id);
          const who = sender?.display_name ?? event.peer_user_id?.slice(0, 12) ?? "a peer";
          notify("Message not delivered", `Couldn't reach ${who}. It hasn't been sent.`, { variant: "error" });
        } else if (event.type === "call_invited") {
          // The backend already auto-declines an invite while we're busy
          // (ringing or already in a call — see `AppService::handle_call_invited`),
          // so seeing this event at all means we're free to ring. Bumping
          // the attempt counter covers the narrow race where this arrives
          // while we're still mid-dial ourselves (our own `callContact()`
          // hasn't resolved yet, so the backend hadn't marked itself busy
          // in time to auto-decline this one): it marks our own dial as
          // superseded, so once it does resolve, `handleStartCall` cleanly
          // hangs it up instead of clobbering the incoming call this event
          // is about to put on screen.
          callAttemptRef.current++;
          stopRingtone();
          ringtoneRef.current = startRingtone(getRingtoneId());
          setActiveCall({ callId: event.call_id, peerUserId: event.from, phase: "incoming" });
        } else if (event.type === "call_accepted") {
          stopRingtone();
          setActiveCall((c) => (c && c.callId === event.call_id ? { ...c, phase: "connected" } : c));
          playVoiceJoinSound();
        } else if (event.type === "call_declined") {
          stopRingtone();
          setActiveCall((c) => (c && c.callId === event.call_id ? null : c));
        } else if (event.type === "call_ended") {
          const wasActive = activeCallRef.current?.callId === event.call_id;
          stopRingtone();
          setActiveCall((c) => (c && c.callId === event.call_id ? null : c));
          if (wasActive) playVoiceLeaveSound();
        } else if (event.type === "call_failed") {
          // Our own `CallInvite`/`CallAccept` never reached them — most
          // commonly because they're not actually online right now despite
          // a still-valid directory presence record (see
          // `ChatNode::pending_call_sends`). The backend's already torn
          // down its side of this call by the time this arrives; cancel
          // the ring screen here too and say why, instead of leaving it
          // stuck on "Calling…" indefinitely.
          if (activeCallRef.current?.callId === event.call_id) {
            stopRingtone();
            setActiveCall(null);
            const who =
              useChatStore.getState().contacts.find((c) => c.user_id === event.peer_user_id)
                ?.display_name ?? event.peer_user_id.slice(0, 12);
            notify("Call failed", `Couldn't reach ${who} — they may not be online.`, { variant: "error" });
          }
        }
      }),
      onGroupsUpdated(() => void refreshGroups()),
      onContactsUpdated(() => void refreshContacts()),
    ];
    return () => {
      unlistenPromises.forEach((p) => p.then((fn) => fn()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userId]);

  useEffect(() => {
    if (!selected) return;
    const id = selected.kind === "dm" ? selected.userId : `${selected.groupId}:${selected.channelId}`;
    setMessagesError(null);
    api
      .listMessages(id)
      .then((msgs) => setMessages(id, msgs))
      .catch((err) => setMessagesError(String(err)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  // Previews who's in each of this group's voice channels before the user
  // joins one — live updates after this come through the
  // "voice_participants_changed" chat-event handler above; this just seeds
  // the initial state whenever a group comes into view (opening it, or a
  // fellow member adding a new voice channel — `groups` changing covers
  // both without needing a separate trigger).
  useEffect(() => {
    if (selected?.kind !== "group") return;
    const group = useChatStore.getState().groups.find((g) => g.group_id === selected.groupId);
    const voiceChannelIds = group?.channels.filter((c) => c.kind === "voice").map((c) => c.channel_id) ?? [];
    for (const channelId of voiceChannelIds) {
      api
        .getChannelVoiceParticipants(channelId)
        .then((userIds) => setChannelVoicePresence(channelId, userIds))
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected, groups]);

  function closeTutorial() {
    localStorage.setItem(TUTORIAL_SEEN_KEY, "1");
    setShowTutorial(false);
  }

  async function handleAddContact(id: string) {
    await api.addContact(id);
    await refreshContacts();
    select({ kind: "dm", userId: id });
  }

  async function handleCreateGroup(name: string) {
    const group = await api.createGroup(name);
    upsertGroup(group);
    // Every new group starts with exactly one channel (the default
    // "general" text channel) — land straight in it.
    const firstChannel = group.channels[0];
    select(firstChannel ? { kind: "group", groupId: group.group_id, channelId: firstChannel.channel_id } : null);
  }

  async function handleInvite(memberUserId: string) {
    if (selected?.kind !== "group") return;
    const group = await api.inviteToGroup(selected.groupId, memberUserId);
    upsertGroup(group);
  }

  async function handleRemoveMember(memberUserId: string) {
    if (selected?.kind !== "group") return;
    const group = await api.removeMemberFromGroup(selected.groupId, memberUserId);
    upsertGroup(group);
  }

  async function handleLeaveGroup() {
    if (selected?.kind !== "group") return;
    const groupId = selected.groupId;
    await api.leaveGroup(groupId);
    select(null);
    await refreshGroups();
  }

  async function handleRemoveContact(contactUserId: string) {
    await api.removeContact(contactUserId);
    if (selected?.kind === "dm" && selected.userId === contactUserId) {
      select(null);
    }
    await refreshContacts();
  }

  async function handleCreateChannel(name: string, kind: ChannelKind) {
    if (selected?.kind !== "group") return;
    const group = await api.createChannel(selected.groupId, name, kind);
    upsertGroup(group);
  }

  async function handlePurge() {
    await api.panicPurge();
    setShowSettings(false);
    await afterAccountRemoved();
  }

  // Shows the ring screen and starts the ringback sound the instant the
  // button is pressed, rather than waiting for `callContact` to resolve —
  // that call does a directory lookup and dial, which can take a while and
  // can fail outright (e.g. the peer's last known address is stale because
  // they're not actually online right now), and pressing "Call" should
  // give some feedback immediately rather than appear to do nothing until
  // it either rings or times out. `callId` starts out as a local
  // placeholder and gets patched to the real one — or, on failure, the
  // whole call is torn back down and the error surfaced — once
  // `callContact` actually settles.
  async function handleStartCall(peerUserId: string) {
    if (activeCall) return; // no call-waiting — matches the backend's own busy-auto-decline
    const attempt = ++callAttemptRef.current;
    const placeholderId = `pending-${attempt}`;
    setActiveCall({ callId: placeholderId, peerUserId, phase: "outgoing", pending: true });
    stopRingtone();
    ringtoneRef.current = startCallingSound();
    try {
      const callId = await api.callContact(peerUserId, getPreferredInputDevice(), getPreferredOutputDevice());
      if (callAttemptRef.current !== attempt) {
        // Cancelled locally, or superseded by an incoming call, while the
        // dial was still in flight — the backend didn't have a real
        // `pending_call` to cancel until just now (see `AppService::call_contact`),
        // so there was nothing to clean up until this resolved.
        api.endCall(callId).catch(() => {});
        return;
      }
      setActiveCall((c) => (c && c.callId === placeholderId ? { callId, peerUserId, phase: "outgoing" } : c));
    } catch (err) {
      if (callAttemptRef.current !== attempt) return; // already cleaned up by whatever superseded this
      stopRingtone();
      setActiveCall((c) => (c && c.callId === placeholderId ? null : c));
      notify("Call failed", String(err), { variant: "error" });
    }
  }

  async function handleAcceptCall() {
    if (!activeCall) return;
    await api.acceptCall(activeCall.callId, getPreferredInputDevice(), getPreferredOutputDevice());
    stopRingtone();
    setActiveCall((c) => (c ? { ...c, phase: "connected" } : c));
  }

  function handleDeclineCall() {
    if (!activeCall) return;
    stopRingtone();
    const callId = activeCall.callId;
    setActiveCall(null);
    api.declineCall(callId).catch(() => {});
  }

  function handleEndCall() {
    if (!activeCall) return;
    stopRingtone();
    callAttemptRef.current++; // invalidates a still-in-flight `callContact()`, if any (see `handleStartCall`)
    const { pending, phase, callId } = activeCall;
    setActiveCall(null);
    if (!pending) {
      // A pending (still-dialing) call has no real `pending_call` on the
      // backend yet to end — `handleStartCall`'s own continuation cleans
      // that up once the dial actually resolves.
      api.endCall(callId).catch(() => {});
    }
    if (phase === "connected") playVoiceLeaveSound();
  }

  if (!splashDone) {
    return <Splash onFinished={() => setSplashDone(true)} />;
  }

  if (phase === "loading") {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-ink">
        <CipherSeal status="connecting" size={40} />
        <p className="text-sm text-text-faint">Waking up…</p>
      </div>
    );
  }

  if (phase === "boot-error") {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-ink px-6 text-center">
        <CipherSeal status="idle" size={32} />
        <p className="max-w-sm text-sm text-text-muted">Couldn't reconnect: {bootError}</p>
        <button
          onClick={handleRetryBoot}
          className="rounded-md bg-brass px-4 py-2 text-sm font-medium text-ink transition hover:scale-105 hover:brightness-110 active:scale-95"
        >
          Try again
        </button>
      </div>
    );
  }

  if (phase === "choose-server") {
    return <ServerChoice onChosen={handleServerChosen} />;
  }

  if (phase === "onboarding") {
    return (
      <Onboarding
        mode={onboardingMode}
        onSubmit={handleCreateAccount}
        onCancel={
          onboardingMode === "add" ? () => setPhase(onboardingReturnPhase) : undefined
        }
      />
    );
  }

  if (phase === "picker") {
    return (
      <AccountPicker
        accounts={accounts}
        busy={pickerBusy}
        error={pickerError}
        onChoose={handleChooseAccount}
        onAddAnother={() => startAddAccount("picker")}
      />
    );
  }

  if (!userId) {
    // Shouldn't be reachable — "ready" is only ever entered right after a
    // successful account load — but fall back to the picker/onboarding
    // decision rather than rendering a broken main view.
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-ink">
        <CipherSeal status="connecting" size={40} />
        <p className="text-sm text-text-faint">Waking up…</p>
      </div>
    );
  }

  const activeGroup = selected?.kind === "group" ? groups.find((g) => g.group_id === selected.groupId) : undefined;
  const activeContact = selected?.kind === "dm" ? contacts.find((c) => c.user_id === selected.userId) : undefined;
  const activeChannel =
    selected?.kind === "group" ? activeGroup?.channels.find((c) => c.channel_id === selected.channelId) : undefined;
  const rail: "dm" | "group" = selected?.kind === "group" ? "group" : "dm";

  let chatConversationId: string | null = null;
  let chatTitle = "";
  let chatSubtitle = "";
  let sealStatus: SealStatus = "idle";
  if (selected?.kind === "dm") {
    chatConversationId = selected.userId;
    chatTitle = activeContact?.display_name ?? selected.userId.slice(0, 12);
    chatSubtitle = "End-to-end encrypted · direct";
    sealStatus = "secure";
  } else if (selected?.kind === "group") {
    chatConversationId = `${selected.groupId}:${selected.channelId}`;
    chatTitle = activeChannel ? `${activeGroup?.name ?? "Group"} · #${activeChannel.name}` : activeGroup?.name ?? "Group";
    chatSubtitle = `End-to-end encrypted · ${activeGroup?.members.length ?? 0} members`;
    sealStatus = "secure";
  }

  function selectFirstChannel(groupId: string) {
    const group = groups.find((g) => g.group_id === groupId);
    const firstTextChannel = group?.channels.find((c) => c.kind === "text") ?? group?.channels[0];
    select(firstTextChannel ? { kind: "group", groupId, channelId: firstTextChannel.channel_id } : null);
    // A fellow member's new channel only reaches us in real time if we were
    // online to catch the announcement (see `refresh_group` on the Rust
    // side) — refetch on open too, so opening a group you've missed that
    // for still shows it up to date instead of only catching up whenever
    // the next announcement happens to arrive.
    api.refreshGroup(groupId).then(upsertGroup).catch(() => {});
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden">
      <Sidebar
        groups={groups}
        selected={selected}
        unread={unread}
        onSelectDms={() => select(contacts[0] ? { kind: "dm", userId: contacts[0].user_id } : null)}
        onSelectGroup={selectFirstChannel}
        onCreateGroup={() => setOpenModal("create-group")}
        onOpenSettings={() => setShowSettings(true)}
      />

      <ConversationList
        mode={rail}
        contacts={contacts}
        selected={selected}
        unread={unread}
        activeGroup={activeGroup}
        voicePresence={voicePresence}
        currentUserId={userId}
        onSelectContact={(userId) => select({ kind: "dm", userId })}
        onSelectGroupChannel={(channelId) =>
          selected?.kind === "group" && select({ kind: "group", groupId: selected.groupId, channelId })
        }
        onAddContact={() => setOpenModal("add-contact")}
        onInvite={() => setOpenModal("invite")}
        onCreateChannel={(kind) => {
          setCreateChannelKind(kind);
          setOpenModal("create-channel");
        }}
        onLeaveGroup={handleLeaveGroup}
        onRemoveContact={handleRemoveContact}
      />

      {selected?.kind === "group" && activeChannel?.kind === "voice" ? (
        <VoiceCallPanel
          key={`${selected.groupId}:${selected.channelId}`}
          groupId={selected.groupId}
          channelId={selected.channelId}
          channelName={activeChannel.name}
          currentUserId={userId!}
        />
      ) : chatConversationId ? (
        <ChatPane
          title={chatTitle}
          subtitle={chatSubtitle}
          sealStatus={sealStatus}
          messages={messagesByConversation[chatConversationId] ?? []}
          currentUserId={userId}
          isGroup={selected?.kind === "group"}
          contacts={contacts}
          loadError={messagesError}
          placeholder="No messages yet. Say hello — it's sealed before it leaves this device."
          onCall={selected?.kind === "dm" ? () => handleStartCall(selected.userId) : undefined}
          onSend={async (body, attachment) => {
            if (selected?.kind === "dm") await api.sendDirectMessage(selected.userId, body, attachment);
            else if (selected?.kind === "group")
              await api.sendGroupMessage(selected.groupId, selected.channelId, body, attachment);
            appendMessage(chatConversationId!, {
              sender_user_id: userId,
              body,
              attachment,
              sent_at: Date.now() / 1000,
            });
          }}
        />
      ) : (
        <EmptyChatPane />
      )}

      {activeGroup && (
        <MemberList group={activeGroup} currentUserId={userId} onRemoveMember={handleRemoveMember} />
      )}

      {openModal === "add-contact" && (
        <Modal
          title="Add someone"
          description="Ask them for their ID from Settings, and enter it here. There's no directory to browse — you connect by exchanging IDs directly, the same way you'd share a phone number."
          fieldLabel="Their ID"
          placeholder="e.g. 4e1c2a9b7f0d3e5a…"
          submitLabel="Add"
          monospaceInput
          onSubmit={handleAddContact}
          onClose={() => setOpenModal(null)}
        />
      )}
      {openModal === "create-group" && (
        <Modal
          title="Create a group"
          fieldLabel="Group name"
          placeholder="e.g. Weekend plans"
          submitLabel="Create"
          onSubmit={handleCreateGroup}
          onClose={() => setOpenModal(null)}
        />
      )}
      {openModal === "invite" && (
        <Modal
          title={`Invite to ${activeGroup?.name ?? "group"}`}
          description="They'll receive the group's encryption key directly from you — never through a server."
          fieldLabel="Their ID"
          placeholder="e.g. 4e1c2a9b7f0d3e5a…"
          submitLabel="Invite"
          monospaceInput
          onSubmit={handleInvite}
          onClose={() => setOpenModal(null)}
        />
      )}
      {openModal === "create-channel" && (
        <CreateChannelModal
          initialKind={createChannelKind}
          onSubmit={handleCreateChannel}
          onClose={() => setOpenModal(null)}
        />
      )}
      {openModal === "update-available" && (
        <Modal
          title="Update Available"
          description="A new update is available. You can update now, leter or never - It's up to you."
          submitLabel="Update"
          onSubmit={(_)=> update()}
          onClose={() => setOpenModal(null)}
        />
      )}
      {openModal === "up-to-date" && (
        <Modal
          title="Up To Date"
          description="There is no new update available, nothing to worry about."
          submitLabel="Ok"
          onSubmit={(_)=> new Promise<void>(()=>{setOpenModal(null)})}
          onClose={() => setOpenModal(null)}
        />
      )}
      {showSettings && (
        <SettingsPanel
          userId={userId}
          displayName={displayName ?? ""}
          onRename={async (name) => {
            await api.renameAccount(name);
            setDisplayName(name);
          }}
          networkStatus={networkStatus}
          savedServerChoice={savedServerChoice}
          onClose={() => setShowSettings(false)}
          onPurge={handlePurge}
          onOpenTutorial={() => setShowTutorial(true)}
          activeAccountId={activeAccountId}
          onSwitchAccount={handleSwitchAccount}
          onRemoveOtherAccount={handleRemoveOtherAccount}
          onAddAnotherAccount={() => startAddAccount("ready")}
          onRemoveCurrentAccount={handleRemoveCurrentAccount}
          setOpenModal={setOpenModal}
          notify={notify}
        />
      )}
      {showTutorial && <TutorialWizard userId={userId} onClose={closeTutorial} />}
      {activeCall && (
        <CallOverlay
          call={activeCall}
          peerDisplayName={
            contacts.find((c) => c.user_id === activeCall.peerUserId)?.display_name ??
            activeCall.peerUserId.slice(0, 12)
          }
          onAccept={handleAcceptCall}
          onDecline={handleDeclineCall}
          onEnd={handleEndCall}
        />
      )}
      <ToastStack toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
