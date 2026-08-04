import type { FormEvent } from "react";
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/tauri";
import type { Attachment, Contact, Message } from "../lib/types";
import { CipherSeal, type SealStatus } from "./CipherSeal";
import { ImageLightbox } from "./ImageLightbox";

interface ChatPaneProps {
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
}: ChatPaneProps) {
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [pendingAttachment, setPendingAttachment] = useState<Attachment | null>(null);
  const [stripExif, setStripExif] = useState(true);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages.length]);

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
    setDraft("");
    setPendingAttachment(null);
    try {
      await onSend(body, attachment);
    } catch (err) {
      // Restore what was cleared optimistically above — a failed send
      // shouldn't cost you the message you typed.
      setDraft(body);
      setPendingAttachment(attachment);
      setSendError(String(err));
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col bg-ink">
      <div className="flex items-center gap-2.5 border-b border-border px-5 py-3.5">
        <CipherSeal status={sealStatus} size={18} />
        <div className="min-w-0">
          <h2 className="truncate font-display text-[15px] font-semibold text-text">{title}</h2>
          <p className="truncate text-xs text-text-faint">{subtitle}</p>
        </div>
      </div>

      <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 py-4">
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
