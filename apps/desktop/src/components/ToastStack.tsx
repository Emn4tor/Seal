export interface ToastItem {
  id: string;
  title: string;
  body: string;
  variant: "message" | "error";
}

interface ToastStackProps {
  toasts: ToastItem[];
  onDismiss: (id: string) => void;
}

/** A reliable, in-window stand-in for OS notifications. Native notifications
 * still get requested too (`lib/notifications.ts`), but on macOS specifically
 * they go through a Tauri dev-mode workaround that impersonates Terminal.app's
 * identity (an unbundled `cargo run` binary has no bundle of its own to
 * notify as) — whether anything actually shows then depends on whether
 * Terminal itself has notification permission, which this app has no way to
 * check or influence. This always shows regardless of OS/permission state. */
export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 top-4 z-[100] flex w-full max-w-sm flex-col gap-2">
      {toasts.map((t) => (
        <button
          key={t.id}
          onClick={() => onDismiss(t.id)}
          className={`toast-enter pointer-events-auto rounded-lg border px-4 py-3 text-left shadow-2xl transition hover:brightness-110 active:scale-[0.99] ${
            t.variant === "error" ? "border-danger-dim/40 bg-surface" : "border-border bg-surface"
          }`}
        >
          <p className={`text-[13px] font-semibold ${t.variant === "error" ? "text-danger" : "text-text"}`}>
            {t.title}
          </p>
          <p className="mt-0.5 line-clamp-2 text-[12.5px] text-text-muted">{t.body}</p>
        </button>
      ))}
    </div>
  );
}
