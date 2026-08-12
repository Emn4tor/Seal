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
  | {
      type: "direct_message";
      message_id: string;
      from: string;
      body: string;
      attachment: Attachment | null;
    }
  | {
      type: "group_message";
      message_id: string;
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
  | { type: "call_failed"; peer_user_id: string; call_id: string; reason: string }
  | { type: "sync_completed"; device_id: string; message_count: number };

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
  /** This account's own directory server (a real URL, or the embedded
   * sentinel) — independent of every other account on this device. */
  directory_url: string;
}

/** Mirrors `accounts::BootDecision` on the Rust side — what to show at startup. */
export type BootDecision =
  | { action: "needsFirstAccount" }
  | { action: "needsPicker"; accounts: AccountSummary[] }
  | { action: "resume"; account: AccountSummary }
  | { action: "createWithName"; display_name: string };

/** Mirrors `dto::PairingOfferDto` — the exact JSON a QR code encodes for
 * another device to scan and join this account with (see `Devices.tsx`).
 * Opaque to the frontend beyond serializing/deserializing it whole. */
export interface PairingOffer {
  user_id: string;
  peer_id: string;
  multiaddrs: string[];
  relay_addrs: string[];
  token: string;
  ephemeral_pubkey_base64: string;
  expires_at: number;
}

/** What the QR code encodes: base64 of the JSON, so it reads as an opaque
 * pairing code. All fields are ASCII, so plain `btoa`/`atob` round-trip cleanly. */
export function encodePairingOffer(offer: PairingOffer): string {
  return btoa(JSON.stringify(offer));
}

export function decodePairingOffer(code: string): PairingOffer {
  return JSON.parse(atob(code.trim())) as PairingOffer;
}

/** Mirrors `dto::DeviceDto` — one row of the "Devices" settings screen. */
export interface Device {
  device_id: string;
  is_this_device: boolean;
  online: boolean;
}
