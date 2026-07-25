import { create } from "zustand";
import type { Contact, Group, Message, NetworkStatus, Selection } from "../lib/types";

interface ChatState {
  userId: string | null;
  displayName: string | null;
  contacts: Contact[];
  groups: Group[];
  messagesByConversation: Record<string, Message[]>;
  selected: Selection;
  unread: Record<string, number>;
  networkStatus: NetworkStatus;

  setUserId: (id: string | null) => void;
  setDisplayName: (name: string | null) => void;
  setContacts: (c: Contact[]) => void;
  setGroups: (g: Group[]) => void;
  upsertGroup: (g: Group) => void;
  setMessages: (conversationId: string, msgs: Message[]) => void;
  appendMessage: (conversationId: string, msg: Message) => void;
  select: (selection: Selection) => void;
  setNetworkStatus: (status: NetworkStatus) => void;
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
    const selected = get().selected;
    const selectedId =
      selected?.kind === "dm" ? selected.userId : selected?.kind === "group" ? `${selected.groupId}:${selected.channelId}` : null;
    const isOpen = selectedId === conversationId;
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
    if (selection) {
      const id = selection.kind === "dm" ? selection.userId : `${selection.groupId}:${selection.channelId}`;
      set((s) => ({ unread: { ...s.unread, [id]: 0 } }));
    }
  },
  reset: () => set({ ...initialState }),
}));
