import { useState } from "react";
import { useModalEscape } from "../lib/useModalEscape";
import { CipherSeal, type SealStatus } from "./CipherSeal";

interface TutorialWizardProps {
  userId: string;
  onClose: () => void;
}

interface Step {
  eyebrow: string;
  title: string;
  body: string;
  seal: SealStatus;
  render?: (userId: string) => React.ReactNode;
}

function ServerDiagram() {
  return (
    <div className="mt-6 flex items-center justify-center gap-4">
      <div className="rounded-lg border border-border bg-ink px-4 py-3 text-center">
        <p className="text-xs font-medium text-text">You</p>
      </div>
      <div className="flex flex-col items-center gap-1">
        <svg width="56" height="16" viewBox="0 0 56 16" fill="none">
          <path d="M0 8h50M44 2l6 6-6 6" stroke="var(--color-verdigris)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <span className="text-[10px] text-verdigris">sealed message</span>
      </div>
      <div className="rounded-lg border border-border bg-ink px-4 py-3 text-center">
        <p className="text-xs font-medium text-text">Them</p>
      </div>
    </div>
  );
}

function ServerAside() {
  return (
    <div className="mx-auto mt-4 flex w-fit items-center gap-2.5 rounded-lg border border-dashed border-border px-3.5 py-2.5 opacity-70">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
        <path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7Z" stroke="var(--color-text-faint)" strokeWidth="1.3" />
        <path d="m4 4 16 16" stroke="var(--color-danger)" strokeWidth="1.3" />
      </svg>
      <span className="text-[11px] text-text-faint">Directory server — sees a name &amp; an address, never this</span>
    </div>
  );
}

function buildSteps(): Step[] {
  return [
    {
      eyebrow: "How Seal works · 1 of 6",
      title: "Keys, not passwords",
      body: "There's no password to remember and no account to recover. When you got started, Seal generated a pair of keys on this device. One half is public — your ID, safe to hand to anyone, like a mailbox address. The other half is private, and it never leaves this device. Anything locked with your public half can only be opened with your private half. There's no export or backup yet, either — losing this device means losing this identity for good.",
      seal: "idle",
      render: (uid) => (
        <div className="mt-6 rounded-lg border border-border bg-ink px-4 py-3 text-center">
          <p className="text-[10px] uppercase tracking-wider text-text-faint">Your public ID</p>
          <code className="mt-1 block break-all font-mono text-[13px] text-brass">{uid}</code>
        </div>
      ),
    },
    {
      eyebrow: "How Seal works · 2 of 6",
      title: "A lock that reinvents itself",
      body: "Every message to a person gets its own lock, and the lock changes after each one. Even if someone captured today's lock, it tells them nothing about yesterday's messages or tomorrow's — each message seals and re-seals the conversation independently. This is the same idea behind the Signal protocol's Double Ratchet.",
      seal: "connecting",
    },
    {
      eyebrow: "How Seal works · 3 of 6",
      title: "A shared broadcast key, for groups",
      body: "Groups work a little differently: everyone in the room holds one shared key, so Seal doesn't have to re-encrypt each message separately for every member. If someone leaves — or gets removed — the group quietly rotates to a new key. From that moment on, they can't read anything said afterward.",
      seal: "secure",
    },
    {
      eyebrow: "How Seal works · 4 of 6",
      title: "What the one server can see",
      body: "Seal talks to exactly one small server: the directory. Its whole job is helping people find each other — it holds a public key, a display name, and, while you're online, a temporary address. There is no code path in this app that sends it a message. Its operator can empty it completely with one click, and nothing of value is lost.",
      seal: "idle",
      render: () => (
        <>
          <ServerDiagram />
          <ServerAside />
        </>
      ),
    },
    {
      eyebrow: "How Seal works · 5 of 6",
      title: "Knowing it's really them",
      body: "Your ID is derived directly from your keys — it can't be forged without also forging the private key that never leaves your device. If you want certainty about who you're talking to, compare their ID with them a second way: in person, or on a call. Once you've confirmed it, you know every sealed message from that ID really came from them.",
      seal: "secure",
    },
    {
      eyebrow: "How Seal works · 6 of 6",
      title: "If you ever need to disappear",
      body: "Settings → Data & Privacy has a button that instantly and permanently destroys everything on this device: your keys, your contacts, your history. It only affects this copy of the app — nobody you've talked to is touched by it — and there's no undo.",
      seal: "idle",
    },
  ];
}

export function TutorialWizard({ userId, onClose }: TutorialWizardProps) {
  const steps = buildSteps();
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const isLast = index === steps.length - 1;

  useModalEscape(onClose);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 px-6">
      <div className="w-full max-w-lg rounded-2xl border border-border bg-surface p-8">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <CipherSeal status={step.seal} size={22} />
            <span className="text-xs font-medium uppercase tracking-wider text-text-faint">{step.eyebrow}</span>
          </div>
          <button
            onClick={onClose}
            aria-label="Close tutorial"
            className="flex h-7 w-7 items-center justify-center rounded-md text-text-faint transition hover:scale-110 hover:bg-surface-raised hover:text-text active:scale-90"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
              <path d="m6 6 12 12M18 6 6 18" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </button>
        </div>

        <h2 className="mt-5 font-display text-2xl font-semibold leading-tight text-text">{step.title}</h2>
        <p className="mt-3 text-[14.5px] leading-relaxed text-text-muted">{step.body}</p>
        {step.render?.(userId)}

        <div className="mt-8 flex items-center justify-between">
          <div className="flex gap-1.5">
            {steps.map((_, i) => (
              <span
                key={i}
                className={`h-1.5 rounded-full transition-all ${
                  i === index ? "w-5 bg-brass" : "w-1.5 bg-border"
                }`}
              />
            ))}
          </div>
          <div className="flex gap-2">
            {index > 0 && (
              <button
                onClick={() => setIndex((i) => i - 1)}
                className="rounded-md px-3.5 py-2 text-sm text-text-muted hover:bg-surface-raised"
              >
                Back
              </button>
            )}
            <button
              onClick={() => (isLast ? onClose() : setIndex((i) => i + 1))}
              className="rounded-md bg-brass px-4 py-2 text-sm font-medium text-ink transition hover:scale-105 hover:brightness-110 active:scale-95"
            >
              {isLast ? "Got it" : "Next"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
