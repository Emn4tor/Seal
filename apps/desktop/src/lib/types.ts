export interface Contact {
  user_id: string;
  display_name: string;
  verified: boolean;
}

export interface Attachment {
  filename: string;
  mime_type: string;
  size: number;
  exif_stripped: boolean;
  data_base64: string;
}

export interface ExifField {
  tag: string;
  value: string;
}

export interface Message {
  sender_user_id: string;
  body: string;
  sent_at: number;
  attachment: Attachment | null;
}

export interface GroupMember {
  user_id: string;
  role: string;
}

export type ChannelKind = "text" | "voice";

export interface Channel {
  channel_id: string;
  name: string;
  kind: ChannelKind;
  position: number;
}

export interface Group {
  group_id: string;
  name: string;
  roster_version: number;
  members: GroupMember[];
  channels: Channel[];
}

export type NetworkStatus = "public" | "private" | "unknown";

export type ChatEvent =
  | { type: "direct_message"; from: string; body: string; attachment: Attachment | null }
  | {
      type: "group_message";
      group_id: string;
      channel_id: string;
      from: string;
      body: string;
      attachment: Attachment | null;
    }
  | { type: "group_key_received"; group_id: string; from: string }
  | { type: "network_status"; status: NetworkStatus }
  | { type: "message_send_failed"; peer_user_id: string | null; reason: string }
  | {
      type: "voice_participants_changed";
      group_id: string;
      channel_id: string;
      user_ids: string[];
    }
  | { type: "call_invited"; from: string; call_id: string }
  | { type: "call_accepted"; from: string; call_id: string }
  | { type: "call_declined"; from: string; call_id: string }
  | { type: "call_ended"; from: string; call_id: string }
  | { type: "call_failed"; peer_user_id: string; call_id: string; reason: string };

export type Selection =
  | { kind: "dm"; userId: string }
  | { kind: "group"; groupId: string; channelId: string }
  | null;

/** The conversation id a `Selection` maps to, the same string
 * `messagesByConversation` and per-conversation drafts are keyed by.
 * `null` when nothing is selected. */
export function conversationIdOf(selection: Selection): string | null {
  if (!selection) return null;
  return selection.kind === "dm" ? selection.userId : `${selection.groupId}:${selection.channelId}`;
}

export interface AccountSummary {
  account_id: string;
  user_id: string;
  display_name: string;
}

/** Mirrors `accounts::BootDecision` on the Rust side — what to show at startup. */
export type BootDecision =
  | { action: "needsFirstAccount" }
  | { action: "needsPicker"; accounts: AccountSummary[] }
  | { action: "resume"; account: AccountSummary }
  | { action: "createWithName"; display_name: string };
