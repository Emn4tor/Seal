import type { AccountSummary } from "../lib/types";
import { CipherSeal } from "./CipherSeal";

interface AccountPickerProps {
  accounts: AccountSummary[];
  busy: boolean;
  error: string | null;
  onChoose: (account: AccountSummary) => void;
  onAddAnother: () => void;
}

export function AccountPicker({ accounts, busy, error, onChoose, onAddAnother }: AccountPickerProps) {
  return (
    <div className="flex h-full items-center justify-center bg-ink px-6">
      <div className="w-full max-w-md">
        <div className="mb-8 flex items-center gap-3">
          <CipherSeal status="secure" size={30} />
          <span className="font-display text-lg font-semibold tracking-tight text-text">Seal</span>
        </div>

        <h1 className="font-display text-2xl font-semibold text-text">Who's this?</h1>
        <p className="mt-2 text-[15px] text-text-muted">
          Pick an account on this device, or add another.
        </p>

        <div className="mt-6 space-y-2">
          {accounts.map((account) => (
            <button
              key={account.account_id}
              disabled={busy}
              onClick={() => onChoose(account)}
              className="flex w-full items-center justify-between rounded-md border border-border bg-surface px-4 py-3 text-left transition enabled:hover:-translate-y-px enabled:hover:border-brass-dim disabled:cursor-not-allowed disabled:opacity-40"
            >
              <span>
                <span className="block text-[15px] font-medium text-text">{account.display_name}</span>
                <span className="mt-0.5 block font-mono text-xs text-text-faint">
                  {account.user_id.slice(0, 16)}…
                </span>
              </span>
              <span className="shrink-0 text-xs font-medium text-brass">Continue</span>
            </button>
          ))}
        </div>

        {error && <p className="mt-3 text-sm text-danger">{error}</p>}

        <button
          onClick={onAddAnother}
          disabled={busy}
          className="mt-4 w-full rounded-md border border-border px-4 py-2.5 text-[15px] font-medium text-brass transition enabled:hover:-translate-y-px enabled:hover:border-brass-dim enabled:hover:bg-brass-wash disabled:cursor-not-allowed disabled:opacity-40"
        >
          Add another account
        </button>
      </div>
    </div>
  );
}
