import type { ReactNode } from "react";
import type { Group, Selection } from "../lib/types";

interface SidebarProps {
  groups: Group[];
  selected: Selection;
  unread: Record<string, number>;
  onSelectDms: () => void;
  onSelectGroup: (groupId: string) => void;
  onCreateGroup: () => void;
  onOpenSettings: () => void;
}

function RailButton({
  active,
  onClick,
  label,
  badge,
  children,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  badge?: number;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      aria-current={active}
      className={`group relative flex h-11 w-11 items-center justify-center rounded-2xl text-sm font-medium transition-all hover:rounded-xl ${
        active
          ? "rounded-xl bg-brass-wash text-brass ring-1 ring-brass/40"
          : "bg-surface text-text-muted hover:bg-surface-raised hover:text-text"
      }`}
    >
      {children}
      {!!badge && (
        <span className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-danger px-1 text-[10px] font-semibold text-text">
          {badge > 9 ? "9+" : badge}
        </span>
      )}
      <span
        className={`absolute -left-3 h-2 w-1 rounded-r-full bg-text transition-all ${
          active ? "h-5 opacity-100" : "opacity-0 group-hover:h-2.5 group-hover:opacity-60"
        }`}
      />
    </button>
  );
}

export function Sidebar({ groups, selected, unread, onSelectDms, onSelectGroup, onCreateGroup, onOpenSettings }: SidebarProps) {
  // Group conversations are keyed "groupId:channelId"; DMs are keyed by a
  // bare user_id (a hex fingerprint, never containing ":") — that's enough
  // to tell the two apart without needing the contact list here too.
  const dmUnread = Object.entries(unread)
    .filter(([id]) => !id.includes(":"))
    .reduce((sum, [, n]) => sum + n, 0);

  function groupUnread(group: Group): number {
    return group.channels.reduce((sum, c) => sum + (unread[`${group.group_id}:${c.channel_id}`] ?? 0), 0);
  }

  return (
    <div className="flex h-full w-[72px] shrink-0 flex-col items-center gap-2 border-r border-border bg-ink py-3">
      <RailButton active={selected?.kind === "dm" || selected === null} onClick={onSelectDms} label="Direct messages" badge={dmUnread}>
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
          <path d="M4 5h16v11H8l-4 4V5Z" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
        </svg>
      </RailButton>

      <div className="my-1 h-px w-8 bg-border" />

      <div className="flex flex-1 flex-col items-center gap-2 overflow-y-auto">
        {groups.map((g) => (
          <RailButton
            key={g.group_id}
            active={selected?.kind === "group" && selected.groupId === g.group_id}
            onClick={() => onSelectGroup(g.group_id)}
            label={g.name}
            badge={groupUnread(g)}
          >
            <span className="font-display text-[15px]">{g.name.trim().slice(0, 1).toUpperCase() || "?"}</span>
          </RailButton>
        ))}

        <button
          onClick={onCreateGroup}
          aria-label="Create a group"
          className="flex h-11 w-11 items-center justify-center rounded-2xl bg-surface text-verdigris transition-all hover:rounded-xl hover:bg-verdigris-wash"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
            <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
        </button>
      </div>

      <button
        onClick={onOpenSettings}
        aria-label="Settings"
        className="flex h-11 w-11 items-center justify-center rounded-2xl bg-surface text-text-muted transition-all hover:rounded-xl hover:bg-surface-raised hover:text-text"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
          <path
            d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"
            stroke="currentColor"
            strokeWidth="1.5"
          />
          <path
            d="M19 12a7 7 0 0 0-.1-1.2l1.9-1.5-2-3.4-2.2.9a7 7 0 0 0-2-1.2L14.3 3H9.7l-.3 2.6a7 7 0 0 0-2 1.2l-2.2-.9-2 3.4L5 10.8a7 7 0 0 0 0 2.4l-1.9 1.5 2 3.4 2.2-.9a7 7 0 0 0 2 1.2l.3 2.6h4.6l.3-2.6a7 7 0 0 0 2-1.2l2.2.9 2-3.4-1.9-1.5c.07-.4.1-.8.1-1.2Z"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinejoin="round"
          />
        </svg>
      </button>
    </div>
  );
}
