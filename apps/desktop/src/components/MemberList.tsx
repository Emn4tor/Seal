import { useState } from "react";
import type { Group } from "../lib/types";

interface MemberListProps {
  group: Group;
  currentUserId: string;
  /** Owner-only in practice (enforced server-side regardless) — omitted
   * entirely for non-owners, same pattern as `ConversationList`'s
   * `canCreate`/`isOwner` gating for channel creation. */
  onRemoveMember?: (userId: string) => Promise<void>;
}

export function MemberList({ group, currentUserId, onRemoveMember }: MemberListProps) {
  const isOwner = group.members.some((m) => m.user_id === currentUserId && m.role === "owner");
  const owners = group.members.filter((m) => m.role === "owner");
  const members = group.members.filter((m) => m.role !== "owner");

  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [removingId, setRemovingId] = useState<string | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);

  async function handleRemove(userId: string) {
    if (!onRemoveMember) return;
    setRemovingId(userId);
    setRemoveError(null);
    try {
      await onRemoveMember(userId);
      setConfirmRemoveId(null);
    } catch (err) {
      setRemoveError(String(err));
    } finally {
      setRemovingId(null);
    }
  }

  const Row = ({ userId, role }: { userId: string; role: string }) => {
    const removable = isOwner && onRemoveMember && userId !== currentUserId;
    return (
      <div className="group/row flex items-center gap-2.5 rounded-md px-2 py-1.5 transition-colors hover:bg-surface-raised/60">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-surface-raised font-display text-[11px] font-medium text-text-muted">
          {userId.slice(0, 1).toUpperCase()}
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13px] text-text">
            {userId === currentUserId ? "You" : `${userId.slice(0, 10)}…`}
          </p>
          {role === "owner" && <p className="text-[10.5px] uppercase tracking-wide text-brass">Owner</p>}
        </div>
        {removable &&
          (confirmRemoveId === userId ? (
            <div className="flex shrink-0 gap-1">
              <button
                onClick={() => setConfirmRemoveId(null)}
                className="rounded px-1.5 py-0.5 text-[11px] text-text-muted hover:bg-surface-raised"
              >
                Cancel
              </button>
              <button
                onClick={() => handleRemove(userId)}
                disabled={removingId === userId}
                className="rounded px-1.5 py-0.5 text-[11px] font-medium text-danger hover:bg-danger/10 disabled:opacity-40"
              >
                {removingId === userId ? "…" : "Confirm"}
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfirmRemoveId(userId)}
              aria-label={`Remove ${userId}`}
              className="shrink-0 rounded px-1.5 py-0.5 text-[11px] text-text-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover/row:opacity-100"
            >
              Remove
            </button>
          ))}
      </div>
    );
  };

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
      {removeError && <p className="mt-3 px-4 text-[11px] text-danger">{removeError}</p>}
    </div>
  );
}
