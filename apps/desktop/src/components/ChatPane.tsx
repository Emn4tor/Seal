import type { FormEvent } from "react";
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/tauri";
import type { Attachment, Contact, Message } from "../lib/types";
import { useChatStore } from "../store/useChatStore";
import { CipherSeal, type SealStatus } from "./CipherSeal";
import { ImageLightbox } from "./ImageLightbox";

interface ChatPaneProps {
  /** The conversation this pane is showing, e.g. a peer's user id for a DM
   * or `groupId:channelId` for a group channel, the same id
   * `messagesByConversation` and `drafts` are keyed by. Used to keep each
   * conversation's draft separate and to discard stale per-conversation UI
   * state (like a send error) when switching to a different one. */
  conversationId: string;
  title: string;
  subtitle: string;
  sealStatus: SealStatus;
  messages: Message[];
  currentUserId: string;
  onSend: (body: string, attachment: Attachment | null) => Promise<void>;
  placeholder: string;
  /** Whether this is a group conversation — sender names are only shown
   * here, never in a DM, where the header already says who you're
   * talking to. */
  isGroup?: boolean;
  /** Only needed when `isGroup` is set, to resolve a sender's display
   * name — a group message only carries `sender_user_id` on the wire. */
  contacts?: Contact[];
  /** Set when this conversation's message history failed to load — shown
   * above whatever messages are already cached, rather than replacing
   * them, since a failed refresh doesn't mean the cached ones are wrong. */
  loadError?: string | null;
  /** Starts a 1:1 voice call with the person this conversation is with —
   * only set (and the header button only shown) for a DM, never a group
   * channel, which has its own voice channels instead. */
  onCall?: () => void;
  /** Whether the person this DM is with is currently blocked, and how to
   * flip that, only set for a DM, same as `onCall`. */
  blocked?: boolean;
  onToggleBlock?: () => Promise<void>;
}

function BlockIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="1.6" />
      <path d="m5.6 5.6 12.8 12.8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function PhoneIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M6.6 10.8c1.4 2.8 3.8 5.2 6.6 6.6l2.2-2.2a1 1 0 0 1 1.1-.2c1.2.5 2.5.7 3.8.7a1 1 0 0 1 1 1V20a1 1 0 0 1-1 1C10.5 21 3 13.5 3 4.7a1 1 0 0 1 1-1h3.3a1 1 0 0 1 1 1c0 1.3.2 2.6.7 3.8a1 1 0 0 1-.2 1.1L6.6 10.8Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function senderName(contacts: Contact[] | undefined, userId: string): string {
  return contacts?.find((c) => c.user_id === userId)?.display_name ?? `${userId.slice(0, 8)}…`;
}

function formatTime(unixSeconds: number) {
  return new Date(unixSeconds * 1000).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function dataUrl(a: Attachment) {
  return `data:${a.mime_type};base64,${a.data_base64}`;
}

function FileIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" className="shrink-0 text-text-muted">
      <path
        d="M6 3h9l5 5v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path d="M14 3v5h5" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
    </svg>
  );
}

function AttachmentBubble({ attachment }: { attachment: Attachment }) {
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const isImage = attachment.mime_type.startsWith("image/");

  async function handleDownload() {
    setDownloadError(null);
    try {
      await api.saveAttachment(attachment.data_base64, attachment.filename);
    } catch (err) {
      setDownloadError(String(err));
    }
  }

  if (isImage) {
    return (
      <>
        <button
          onClick={() => setLightboxOpen(true)}
          className="group block max-w-[240px] overflow-hidden rounded-lg border border-border transition hover:border-brass-dim"
        >
          <img
            src={dataUrl(attachment)}
            alt={attachment.filename}
            className="block max-h-[240px] w-full object-cover transition-transform duration-300 group-hover:scale-105"
          />
        </button>
        {lightboxOpen && <ImageLightbox attachment={attachment} onClose={() => setLightboxOpen(false)} />}
      </>
    );
  }

  return (
    <div className="flex items-center gap-2.5 rounded-lg border border-border bg-ink px-3 py-2.5">
      <FileIcon />
      <div className="min-w-0 flex-1">
        <p className="truncate text-[13px] font-medium text-text">{attachment.filename}</p>
        <p className="text-[11px] text-text-faint">{formatSize(attachment.size)}</p>
        {downloadError && <p className="text-[11px] text-danger">{downloadError}</p>}
      </div>
      <button
        onClick={handleDownload}
        className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-brass hover:bg-brass-wash active:scale-95"
      >
        Save
      </button>
    </div>
  );
}

export function ChatPane({
  conversationId,
  title,
  subtitle,
  sealStatus,
  messages,
  currentUserId,
  onSend,
  placeholder,
  isGroup,
  contacts,
  loadError,
  onCall,
  blocked,
  onToggleBlock,
}: ChatPaneProps) {
  const draft = useChatStore((s) => s.drafts[conversationId]?.body ?? "");
  const pendingAttachment = useChatStore((s) => s.drafts[conversationId]?.attachment ?? null);
  const setDraftInStore = useChatStore((s) => s.setDraft);
  const clearDraftInStore = useChatStore((s) => s.clearDraft);
  const [sending, setSending] = useState(false);
  const [stripExif, setStripExif] = useState(true);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [blockBusy, setBlockBusy] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  function setDraft(body: string) {
    setDraftInStore(conversationId, { body, attachment: pendingAttachment });
  }

  function setPendingAttachment(attachment: Attachment | null) {
    setDraftInStore(conversationId, { body: draft, attachment });
  }

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages.length]);

  // A conversation's own draft/attachment persists in the store (see
  // `useChatStore`); only the transient, attempt-scoped error state below
  // is local, and that should reset rather than carry over when switching
  // to a different conversation.
  useEffect(() => {
    setAttachError(null);
    setSendError(null);
  }, [conversationId]);

  async function handleAttach() {
    setAttachError(null);
    try {
      const attachment = await api.pickAttachment(stripExif);
      if (attachment) setPendingAttachment(attachment);
    } catch (err) {
      setAttachError(String(err));
    }
  }

  async function handleSend(e: FormEvent) {
    e.preventDefault();
    const body = draft.trim();
    if ((!body && !pendingAttachment) || sending) return;
    setSending(true);
    setSendError(null);
    const attachment = pendingAttachment;
    const sentToConversationId = conversationId;
    clearDraftInStore(sentToConversationId);
    try {
      await onSend(body, attachment);
    } catch (err) {
      // Restore what was cleared optimistically above — a failed send
      // shouldn't cost you the message you typed. Restored into the
      // conversation it was actually written for, not whichever one
      // happens to be open by the time this rejects.
      setDraftInStore(sentToConversationId, { body, attachment });
      setSendError(String(err));
    } finally {
      setSending(false);
    }
  }

  async function handleToggleBlock() {
    if (!onToggleBlock) return;
    setBlockBusy(true);
    try {
      await onToggleBlock();
    } finally {
      setBlockBusy(false);
    }
  }

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col bg-ink">
      <div className="flex items-center gap-2.5 border-b border-border px-5 py-3.5">
        <CipherSeal status={sealStatus} size={18} />
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-display text-[15px] font-semibold text-text">{title}</h2>
          <p className="truncate text-xs text-text-faint">{subtitle}</p>
        </div>
        {onCall && (
          <button
            onClick={onCall}
            aria-label="Start a voice call"
            title="Start a voice call"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-verdigris transition hover:scale-110 hover:bg-verdigris-wash active:scale-90"
          >
            <PhoneIcon />
          </button>
        )}
        {onToggleBlock && (
          <button
            onClick={handleToggleBlock}
            disabled={blockBusy}
            aria-pressed={blocked ?? false}
            aria-label={blocked ? "Unblock this person" : "Block this person"}
            title={blocked ? "Unblock this person" : "Block this person"}
            className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition hover:scale-110 active:scale-90 disabled:opacity-60 ${
              blocked ? "bg-danger-wash text-danger" : "text-text-muted hover:bg-surface-raised"
            }`}
          >
            <BlockIcon />
          </button>
        )}
      </div>

      <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 py-4">
        {blocked && (
          <p className="mb-2 rounded-lg border border-danger-dim bg-danger-wash px-3 py-2 text-center text-xs text-danger">
            This person is blocked. Their messages won't reach you until you unblock them.
          </p>
        )}
        {loadError && <p className="mb-2 text-center text-xs text-danger">{loadError}</p>}
        {messages.length === 0 && (
          <div className="flex h-full items-center justify-center">
            <p className="max-w-xs text-center text-sm text-text-faint">{placeholder}</p>
          </div>
        )}
        <div className="flex flex-col gap-3">
          {messages.map((m, i) => {
            const mine = m.sender_user_id === currentUserId;
            return (
              <div key={i} className={`flex ${mine ? "justify-end" : "justify-start"}`}>
                <div className={`max-w-[65%] ${mine ? "items-end" : "items-start"} flex flex-col gap-1.5`}>
                  {!mine && isGroup && (
                    <span className="px-1 text-[12px] font-medium text-text-muted">
                      {senderName(contacts, m.sender_user_id)}
                    </span>
                  )}
                  {m.attachment && <AttachmentBubble attachment={m.attachment} />}
                  {m.body && (
                    <div
                      className={`rounded-2xl px-3.5 py-2 text-[14px] leading-relaxed ${
                        mine
                          ? "rounded-br-sm bg-verdigris-wash text-text"
                          : "rounded-bl-sm bg-surface text-text"
                      }`}
                    >
                      {m.body}
                    </div>
                  )}
                  <span className="px-1 font-mono text-[10.5px] text-text-faint">{formatTime(m.sent_at)}</span>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      <form onSubmit={handleSend} className="border-t border-border px-5 py-3.5">
        {pendingAttachment && (
          <div className="mb-2 flex items-center gap-2.5 rounded-lg border border-border bg-surface px-3 py-2">
            {pendingAttachment.mime_type.startsWith("image/") ? (
              <img src={dataUrl(pendingAttachment)} alt="" className="h-9 w-9 shrink-0 rounded object-cover" />
            ) : (
              <FileIcon />
            )}
            <div className="min-w-0 flex-1">
              <p className="truncate text-[13px] font-medium text-text">{pendingAttachment.filename}</p>
              <p className="text-[11px] text-text-faint">{formatSize(pendingAttachment.size)}</p>
            </div>
            <button
              type="button"
              onClick={() => setPendingAttachment(null)}
              aria-label="Remove attachment"
              className="shrink-0 rounded-md px-2 py-1 text-xs text-text-muted hover:bg-surface-raised"
            >
              Remove
            </button>
          </div>
        )}
        {attachError && <p className="mb-2 text-xs text-danger">{attachError}</p>}
        {sendError && <p className="mb-2 text-xs text-danger">{sendError}</p>}
        <div className="flex items-center gap-2 rounded-xl border border-border bg-surface px-3.5 py-2 transition-colors focus-within:border-brass-dim">
          <button
            type="button"
            onClick={handleAttach}
            aria-label="Attach a file"
            title="Attach a file"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-text-muted transition hover:scale-110 hover:bg-surface-raised hover:text-text active:scale-90"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path
                d="M17 8.5 9.5 16a3 3 0 1 1-4.24-4.24l7.78-7.78a2 2 0 1 1 2.83 2.83L8.4 14.34a1 1 0 1 1-1.41-1.41l6.36-6.36"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Write a sealed message…"
            className="flex-1 bg-transparent text-[14px] text-text placeholder:text-text-faint focus:outline-none"
          />
          <button
            type="submit"
            disabled={(!draft.trim() && !pendingAttachment) || sending}
            aria-label="Send"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-brass text-ink transition enabled:hover:scale-110 enabled:active:scale-90 disabled:cursor-not-allowed disabled:opacity-30"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
              <path d="M4 12h16M13 5l7 7-7 7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>
        <label className="mt-2 flex w-fit items-center gap-1.5 text-[11px] text-text-faint">
          <input
            type="checkbox"
            checked={stripExif}
            onChange={(e) => setStripExif(e.target.checked)}
            className="h-3 w-3 accent-brass"
          />
          Strip photo metadata (location, camera info) before sending
        </label>
      </form>
    </div>
  );
}

export function EmptyChatPane() {
  return (
    <div className="flex h-full flex-1 flex-col items-center justify-center gap-3 bg-ink">
      <CipherSeal status="idle" size={40} />
      <p className="text-sm text-text-faint">Pick a conversation, or add someone to start one.</p>
    </div>
  );
}
