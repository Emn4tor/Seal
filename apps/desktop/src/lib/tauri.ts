import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountSummary,
  Attachment,
  BootDecision,
  ChannelKind,
  ChatEvent,
  Contact,
  ExifField,
  Group,
  Message,
} from "./types";

/** Mirrors `server_config::EMBEDDED_SENTINEL` on the Rust side. */
export const EMBEDDED_SERVER_SENTINEL = "embedded";

export interface AccountsState {
  accounts: AccountSummary[];
  active_account_id: string | null;
}

export const api = {
  getOfficialServerUrl: () => invoke<string | null>("get_official_server_url"),
  getSavedServerUrl: () => invoke<string | null>("get_saved_server_url"),
  startBackend: (serverUrl: string) => invoke<string>("start_backend", { serverUrl }),
  saveServerUrlForNextLaunch: (serverUrl: string) =>
    invoke<void>("save_server_url_for_next_launch", { serverUrl }),
  resolveBootAccount: () => invoke<BootDecision>("resolve_boot_account"),
  listAccounts: () => invoke<AccountsState>("list_accounts"),
  createAccount: (serverUrl: string, displayName: string) =>
    invoke<AccountSummary>("create_account", { serverUrl, displayName }),
  resumeAccount: (serverUrl: string, accountId: string) =>
    invoke<AccountSummary>("resume_account", { serverUrl, accountId }),
  renameAccount: (newDisplayName: string) =>
    invoke<void>("rename_account", { newDisplayName }),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),
  addContact: (userId: string) => invoke<void>("add_contact", { userId }),
  removeContact: (userId: string) => invoke<void>("remove_contact", { userId }),
  listContacts: () => invoke<Contact[]>("list_contacts"),
  sendDirectMessage: (peerUserId: string, body: string, attachment: Attachment | null) =>
    invoke<void>("send_direct_message", { peerUserId, body, attachment }),
  listMessages: (conversationId: string) => invoke<Message[]>("list_messages", { conversationId }),
  createGroup: (name: string) => invoke<Group>("create_group", { name }),
  inviteToGroup: (groupId: string, memberUserId: string) =>
    invoke<Group>("invite_to_group", { groupId, memberUserId }),
  removeMemberFromGroup: (groupId: string, memberUserId: string) =>
    invoke<Group>("remove_member_from_group", { groupId, memberUserId }),
  leaveGroup: (groupId: string) => invoke<void>("leave_group", { groupId }),
  createChannel: (groupId: string, name: string, kind: ChannelKind) =>
    invoke<Group>("create_channel", { groupId, name, kind }),
  sendGroupMessage: (
    groupId: string,
    channelId: string,
    body: string,
    attachment: Attachment | null,
  ) => invoke<void>("send_group_message", { groupId, channelId, body, attachment }),
  listGroups: () => invoke<Group[]>("list_groups"),
  refreshGroup: (groupId: string) => invoke<Group>("refresh_group", { groupId }),
  joinVoiceChannel: (
    groupId: string,
    channelId: string,
    preferredInput: string | null,
    preferredOutput: string | null,
  ) => invoke<void>("join_voice_channel", { groupId, channelId, preferredInput, preferredOutput }),
  leaveVoiceChannel: () => invoke<void>("leave_voice_channel"),
  callContact: (peerUserId: string, preferredInput: string | null, preferredOutput: string | null) =>
    invoke<string>("call_contact", { peerUserId, preferredInput, preferredOutput }),
  acceptCall: (callId: string, preferredInput: string | null, preferredOutput: string | null) =>
    invoke<void>("accept_call", { callId, preferredInput, preferredOutput }),
  declineCall: (callId: string) => invoke<void>("decline_call", { callId }),
  endCall: (callId: string) => invoke<void>("end_call", { callId }),
  listInputDevices: () => invoke<string[]>("list_input_devices"),
  listOutputDevices: () => invoke<string[]>("list_output_devices"),
  setVoiceChangerEnabled: (enabled: boolean) =>
    invoke<void>("set_voice_changer_enabled", { enabled }),
  setMicMuted: (muted: boolean) => invoke<void>("set_mic_muted", { muted }),
  getMicMuted: () => invoke<boolean>("get_mic_muted"),
  getVoiceParticipants: () => invoke<string[]>("get_voice_participants"),
  getChannelVoiceParticipants: (channelId: string) =>
    invoke<string[]>("get_channel_voice_participants", { channelId }),
  setMicThresholdDb: (db: number) => invoke<void>("set_mic_threshold_db", { db }),
  setHearSelf: (enabled: boolean) => invoke<void>("set_hear_self", { enabled }),
  getVoiceSpeakingParticipants: () => invoke<string[]>("get_voice_speaking_participants"),
  panicPurge: () => invoke<void>("panic_purge"),
  pickAttachment: (stripExif: boolean) =>
    invoke<Attachment | null>("pick_attachment", { stripExif }),
  getImageExif: (dataBase64: string) =>
    invoke<ExifField[]>("get_image_exif", { dataBase64 }),
  saveAttachment: (dataBase64: string, suggestedFilename: string) =>
    invoke<boolean>("save_attachment", { dataBase64, suggestedFilename }),
};

export function onChatEvent(handler: (event: ChatEvent) => void): Promise<UnlistenFn> {
  return listen<ChatEvent>("chat-event", (e) => handler(e.payload));
}

export function onGroupsUpdated(handler: () => void): Promise<UnlistenFn> {
  return listen("groups-updated", () => handler());
}

export function onContactsUpdated(handler: () => void): Promise<UnlistenFn> {
  return listen("contacts-updated", () => handler());
}

/** Fired only when the tray menu's mic-mute item changes mute state — the
 * one mute-state change with no JS caller already aware of the result
 * (see `actor.rs`'s `Command::ToggleMicMuted`). Not fired for the manual
 * in-call mute button or push-to-talk, which already know their own
 * outcome without a round trip. */
export function onMicMutedChanged(handler: (muted: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>("mic-muted-changed", (e) => handler(e.payload));
}
