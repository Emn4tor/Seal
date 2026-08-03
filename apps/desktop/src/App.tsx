import { useEffect, useState } from "react";
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
import { TutorialWizard } from "./components/TutorialWizard";
import { CipherSeal, type SealStatus } from "./components/CipherSeal";
import { VoiceCallPanel } from "./components/VoiceCallPanel";
import { applyPushToTalk, getPttEnabled, getPttShortcut } from "./lib/pushToTalk";
import { getAutostartEnabled, syncAutostart } from "./lib/autostart";
import { ensureNotificationPermission, notifyNewMessage } from "./lib/notifications";
import type { AccountSummary, ChannelKind } from "./lib/types";

type OpenModal = "add-contact" | "create-group" | "invite" | "create-channel" | null;
type Phase = "loading" | "boot-error" | "choose-server" | "onboarding" | "picker" | "ready";
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
  const [bootError, setBootError] = useState<string | null>(null);
  const [serverUrl, setServerUrl] = useState<string | null>(null);
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
    reset: resetChatStore,
  } = useChatStore();

  async function refreshContacts() {
    setContacts(await api.listContacts());
  }
  async function refreshGroups() {
    setGroups(await api.listGroups());
  }

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
          if (!wasOpen) {
            const sender = useChatStore.getState().contacts.find((c) => c.user_id === event.from);
            notifyNewMessage(
              sender?.display_name ?? event.from.slice(0, 12),
              event.body || (event.attachment ? "Sent an attachment" : ""),
            );
          }
        } else if (event.type === "group_message") {
          const conversationId = `${event.group_id}:${event.channel_id}`;
          const wasOpen = isConversationOpen(conversationId);
          appendMessage(conversationId, {
            sender_user_id: event.from,
            body: event.body,
            attachment: event.attachment,
            sent_at: Date.now() / 1000,
          });
          if (!wasOpen) {
            const state = useChatStore.getState();
            const group = state.groups.find((g) => g.group_id === event.group_id);
            const channel = group?.channels.find((c) => c.channel_id === event.channel_id);
            const sender = state.contacts.find((c) => c.user_id === event.from);
            const senderName = sender?.display_name ?? event.from.slice(0, 12);
            const bodyText = event.body || (event.attachment ? "Sent an attachment" : "");
            notifyNewMessage(
              group ? (channel ? `${group.name} · #${channel.name}` : group.name) : "Group message",
              `${senderName}: ${bodyText}`,
            );
          }
        } else if (event.type === "network_status") {
          setNetworkStatus(event.status);
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
    api.listMessages(id).then((msgs) => setMessages(id, msgs));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

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

  if (phase === "loading") {
    return (
      <div className="flex h-screen items-center justify-center bg-ink">
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
          className="rounded-md bg-brass px-4 py-2 text-sm font-medium text-ink transition hover:brightness-110"
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
      <div className="flex h-screen items-center justify-center bg-ink">
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
          placeholder="No messages yet. Say hello — it's sealed before it leaves this device."
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

      {activeGroup && <MemberList group={activeGroup} currentUserId={userId} />}

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
      {showSettings && (
        <SettingsPanel
          userId={userId}
          displayName={displayName ?? ""}
          onRename={async (name) => {
            await api.renameAccount(name);
            setDisplayName(name);
          }}
          networkStatus={networkStatus}
          serverUrl={serverUrl}
          onClose={() => setShowSettings(false)}
          onPurge={handlePurge}
          onOpenTutorial={() => setShowTutorial(true)}
          activeAccountId={activeAccountId}
          onSwitchAccount={handleSwitchAccount}
          onRemoveOtherAccount={handleRemoveOtherAccount}
          onAddAnotherAccount={() => startAddAccount("ready")}
          onRemoveCurrentAccount={handleRemoveCurrentAccount}
        />
      )}
      {showTutorial && <TutorialWizard userId={userId} onClose={closeTutorial} />}
    </div>
  );
}
