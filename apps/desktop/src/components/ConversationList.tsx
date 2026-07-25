import type { Channel, Contact, Group, Selection } from "../lib/types";
import { CipherSeal } from "./CipherSeal";

interface ConversationListProps {
  mode: "dm" | "group";
  contacts: Contact[];
  selected: Selection;
  unread: Record<string, number>;
  activeGroup?: Group;
  currentUserId: string;
  onSelectContact: (userId: string) => void;
  onSelectGroupChannel: (channelId: string) => void;
  onAddContact: () => void;
  onInvite: () => void;
  onCreateChannel: (kind: "text" | "voice") => void;
}

function Avatar({ name }: { name: string }) {
  return (
    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-surface-raised font-display text-xs font-medium text-text-muted">
      {name.trim().slice(0, 1).toUpperCase() || "?"}
    </div>
  );
}

function VoiceIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" className="text-text-faint">
      <path
        d="M11 5 6 9H3v6h3l5 4V5Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path d="M16 8a5 5 0 0 1 0 8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function ChannelSection({
  title,
  channels,
  selected,
  canCreate,
  onSelect,
  onCreate,
}: {
  title: string;
  channels: Channel[];
  selected: Selection;
  canCreate: boolean;
  onSelect: (channelId: string) => void;
  onCreate: () => void;
}) {
  return (
    <div className="mb-4">
      <div className="mb-1 flex items-center justify-between px-2.5">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-text-faint">{title}</span>
        {canCreate && (
          <button
            onClick={onCreate}
            aria-label={`Add a ${title.toLowerCase()} channel`}
            className="flex h-4 w-4 items-center justify-center rounded text-text-faint hover:bg-surface-raised hover:text-brass"
          >
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none">
              <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </div>
      {channels.map((channel) => {
        const active = selected?.kind === "group" && selected.channelId === channel.channel_id;
        return (
          <button
            key={channel.channel_id}
            onClick={() => onSelect(channel.channel_id)}
            className={`mb-0.5 flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition ${
              active ? "bg-surface-raised text-text" : "text-text-muted hover:bg-surface-raised/60 hover:text-text"
            }`}
          >
            {channel.kind === "voice" ? <VoiceIcon /> : <span className="text-text-faint">#</span>}
            <span className="truncate">{channel.name}</span>
          </button>
        );
      })}
    </div>
  );
}

export function ConversationList({
  mode,
  contacts,
  selected,
  unread,
  activeGroup,
  currentUserId,
  onSelectContact,
  onSelectGroupChannel,
  onAddContact,
  onInvite,
  onCreateChannel,
}: ConversationListProps) {
  if (mode === "group") {
    const isOwner = activeGroup?.members.some((m) => m.user_id === currentUserId && m.role === "owner") ?? false;
    const textChannels = activeGroup?.channels.filter((c) => c.kind === "text") ?? [];
    const voiceChannels = activeGroup?.channels.filter((c) => c.kind === "voice") ?? [];

    return (
      <div className="flex h-full w-64 shrink-0 flex-col border-r border-border bg-surface">
        <div className="border-b border-border px-4 py-4">
          <h2 className="truncate font-display text-[15px] font-semibold text-text">{activeGroup?.name ?? "Group"}</h2>
          <p className="mt-0.5 text-xs text-text-faint">{activeGroup?.members.length ?? 0} members</p>
        </div>
        <div className="flex-1 overflow-y-auto px-2 py-3">
          <ChannelSection
            title="Text channels"
            channels={textChannels}
            selected={selected}
            canCreate={isOwner}
            onSelect={onSelectGroupChannel}
            onCreate={() => onCreateChannel("text")}
          />
          <ChannelSection
            title="Voice channels"
            channels={voiceChannels}
            selected={selected}
            canCreate={isOwner}
            onSelect={onSelectGroupChannel}
            onCreate={() => onCreateChannel("voice")}
          />
        </div>
        <div className="border-t border-border p-3">
          <button
            onClick={onInvite}
            className="w-full rounded-md border border-border py-2 text-xs font-medium text-text-muted transition hover:border-brass-dim hover:text-brass"
          >
            Invite people
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full w-64 shrink-0 flex-col border-r border-border bg-surface">
      <div className="flex items-center justify-between border-b border-border px-4 py-4">
        <h2 className="font-display text-[15px] font-semibold text-text">Direct messages</h2>
        <button
          onClick={onAddContact}
          aria-label="Add a contact"
          className="flex h-6 w-6 items-center justify-center rounded-md text-text-muted hover:bg-surface-raised hover:text-brass"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
          </svg>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-2 py-2">
        {contacts.length === 0 && (
          <div className="px-2.5 py-6 text-center">
            <p className="text-sm text-text-muted">No conversations yet.</p>
            <button onClick={onAddContact} className="mt-2 text-xs font-medium text-brass hover:underline">
              Add someone by their ID
            </button>
          </div>
        )}
        {contacts.map((c) => {
          const active = selected?.kind === "dm" && selected.userId === c.user_id;
          const unreadCount = unread[c.user_id] ?? 0;
          return (
            <button
              key={c.user_id}
              onClick={() => onSelectContact(c.user_id)}
              className={`mb-0.5 flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition ${
                active ? "bg-surface-raised" : "hover:bg-surface-raised/60"
              }`}
            >
              <Avatar name={c.display_name} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-[13.5px] font-medium text-text">{c.display_name}</span>
                  {c.verified && <CipherSeal status="secure" size={12} />}
                </div>
                <p className="truncate font-mono text-[11px] text-text-faint">{c.user_id.slice(0, 12)}…</p>
              </div>
              {unreadCount > 0 && (
                <span className="flex h-4.5 min-w-4.5 items-center justify-center rounded-full bg-danger px-1 text-[10px] font-semibold text-text">
                  {unreadCount}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
