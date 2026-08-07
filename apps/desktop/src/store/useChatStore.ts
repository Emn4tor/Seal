import { create } from "zustand";
import { conversationIdOf } from "../lib/types";
import type { Attachment, Contact, Group, Message, NetworkStatus, Selection } from "../lib/types";

export interface ConversationDraft {
  body: string;
  attachment: Attachment | null;
}

interface ChatState {
  userId: string | null;
  displayName: string | null;
  contacts: Contact[];
  groups: Group[];
  messagesByConversation: Record<string, Message[]>;
  selected: Selection;
  unread: Record<string, number>;
  networkStatus: NetworkStatus;
  /** Who's in each voice channel right now, keyed by channel_id — kept for
   * every channel we know about (not just one we've joined), so the
   * channel list can preview who's in a call before joining it. */
  voicePresence: Record<string, string[]>;
  /** An in-progress, unsent message per conversation, keyed the same way
   * as `messagesByConversation`. Lives here instead of local `ChatPane`
   * state so it survives switching away and back, and so `ChatPane` never
   * sends whatever's typed to whichever conversation happens to be
   * selected when Send is pressed rather than the one it was typed in. */
  drafts: Record<string, ConversationDraft>;

  setUserId: (id: string | null) => void;
  setDisplayName: (name: string | null) => void;
  setContacts: (c: Contact[]) => void;
  setGroups: (g: Group[]) => void;
  upsertGroup: (g: Group) => void;
  setMessages: (conversationId: string, msgs: Message[]) => void;
  appendMessage: (conversationId: string, msg: Message) => void;
  select: (selection: Selection) => void;
  setNetworkStatus: (status: NetworkStatus) => void;
  setChannelVoicePresence: (channelId: string, userIds: string[]) => void;
  setDraft: (conversationId: string, draft: ConversationDraft) => void;
  clearDraft: (conversationId: string) => void;
  /** Clears everything account-scoped — call when switching to a different account. */
  reset: () => void;
}

const initialState = {
  userId: null,
  displayName: null,
  contacts: [],
  groups: [],
  messagesByConversation: {},
  selected: null,
  unread: {},
  networkStatus: "unknown" as NetworkStatus,
  voicePresence: {},
  drafts: {},
};

export const useChatStore = create<ChatState>((set, get) => ({
  ...initialState,

  setUserId: (id) => set({ userId: id }),
  setDisplayName: (name) => set({ displayName: name }),
  setNetworkStatus: (status) => set({ networkStatus: status }),
  setContacts: (contacts) => set({ contacts }),
  setGroups: (groups) => set({ groups }),
  upsertGroup: (g) =>
    set((s) => ({
      groups: s.groups.some((x) => x.group_id === g.group_id)
        ? s.groups.map((x) => (x.group_id === g.group_id ? g : x))
        : [...s.groups, g],
    })),
  setMessages: (conversationId, msgs) =>
    set((s) => ({ messagesByConversation: { ...s.messagesByConversation, [conversationId]: msgs } })),
  appendMessage: (conversationId, msg) => {
    const isOpen = conversationIdOf(get().selected) === conversationId;
    set((s) => ({
      messagesByConversation: {
        ...s.messagesByConversation,
        [conversationId]: [...(s.messagesByConversation[conversationId] ?? []), msg],
      },
      unread: isOpen ? s.unread : { ...s.unread, [conversationId]: (s.unread[conversationId] ?? 0) + 1 },
    }));
  },
  select: (selection) => {
    set({ selected: selection });
    const id = conversationIdOf(selection);
    if (id) set((s) => ({ unread: { ...s.unread, [id]: 0 } }));
  },
  setChannelVoicePresence: (channelId, userIds) =>
    set((s) => ({ voicePresence: { ...s.voicePresence, [channelId]: userIds } })),
  setDraft: (conversationId, draft) =>
    set((s) => ({ drafts: { ...s.drafts, [conversationId]: draft } })),
  clearDraft: (conversationId) =>
    set((s) => {
      const drafts = { ...s.drafts };
      delete drafts[conversationId];
      return { drafts };
    }),
  reset: () => set({ ...initialState }),
}));
