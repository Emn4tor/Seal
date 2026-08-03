import type { Group } from "../lib/types";

interface MemberListProps {
  group: Group;
  currentUserId: string;
}

export function MemberList({ group, currentUserId }: MemberListProps) {
  const owners = group.members.filter((m) => m.role === "owner");
  const members = group.members.filter((m) => m.role !== "owner");

  const Row = ({ userId, role }: { userId: string; role: string }) => (
    <div className="flex items-center gap-2.5 rounded-md px-2 py-1.5 transition-colors hover:bg-surface-raised/60">
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-surface-raised font-display text-[11px] font-medium text-text-muted">
        {userId.slice(0, 1).toUpperCase()}
      </div>
      <div className="min-w-0">
        <p className="truncate text-[13px] text-text">
          {userId === currentUserId ? "You" : `${userId.slice(0, 10)}…`}
        </p>
        {role === "owner" && <p className="text-[10.5px] uppercase tracking-wide text-brass">Owner</p>}
      </div>
    </div>
  );

  return (
    <div className="hidden h-full w-56 shrink-0 flex-col border-l border-border bg-surface py-4 lg:flex">
      <h3 className="px-4 pb-2 text-[11px] font-semibold uppercase tracking-wider text-text-faint">
        Owner — {owners.length}
      </h3>
      <div className="px-2">
        {owners.map((m) => (
          <Row key={m.user_id} userId={m.user_id} role={m.role} />
        ))}
      </div>
      {members.length > 0 && (
        <>
          <h3 className="mt-4 px-4 pb-2 text-[11px] font-semibold uppercase tracking-wider text-text-faint">
            Members — {members.length}
          </h3>
          <div className="px-2">
            {members.map((m) => (
              <Row key={m.user_id} userId={m.user_id} role={m.role} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
