import { useState } from "react";
import type {
  Account,
  AccountTypeId,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  Currency,
  Jurisdiction,
} from "../types/finance";

const ACCOUNT_TYPES: { id: AccountTypeId; label: string }[] = [
  { id: "chequing", label: "Chequing" },
  { id: "savings", label: "Savings" },
  { id: "brokerage", label: "Brokerage" },
  { id: "tfsa", label: "TFSA" },
  { id: "rrsp", label: "RRSP" },
  { id: "fhsa", label: "FHSA" },
  { id: "401k", label: "401(k)" },
  { id: "ira", label: "IRA" },
  { id: "roth_ira", label: "Roth IRA" },
  { id: "credit", label: "Credit Card" },
  { id: "crypto", label: "Crypto" },
  { id: "other", label: "Other" },
];

type ModalMode = "add_account" | "update_balance";

interface Props {
  isOpen: boolean;
  mode: ModalMode;
  accountToUpdate?: Account | null;
  onClose: () => void;
  onAddAccount: (payload: AddAccountPayload) => Promise<void>;
  onUpdateBalance: (payload: AddBalanceSnapshotPayload) => Promise<void>;
}

const today = () => new Date().toISOString().slice(0, 10);

export default function AccountModal({
  isOpen,
  mode,
  accountToUpdate,
  onClose,
  onAddAccount,
  onUpdateBalance,
}: Props) {
  const [name, setName] = useState("");
  const [institution, setInstitution] = useState("");
  const [accountType, setAccountType] = useState<AccountTypeId>("savings");
  const [currency, setCurrency] = useState<Currency>("USD");
  const [jurisdiction, setJurisdiction] = useState<Jurisdiction>("US");
  const [notes, setNotes] = useState("");

  const [balance, setBalance] = useState("");
  const [snapshotDate, setSnapshotDate] = useState(today());

  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      if (mode === "add_account") {
        await onAddAccount({
          name: name.trim(),
          institution: institution.trim(),
          account_type: accountType,
          currency,
          jurisdiction,
          notes: notes.trim() || null,
        });
      } else if (mode === "update_balance" && accountToUpdate) {
        const val = parseFloat(balance);
        if (isNaN(val)) {
          setError("Enter a valid number for the balance.");
          setSubmitting(false);
          return;
        }
        await onUpdateBalance({
          account_id: accountToUpdate.id,
          balance: val,
          snapshot_date: snapshotDate,
        });
      }
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="w-full max-w-md rounded-2xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
        <h2 className="text-lg font-semibold text-white mb-5">
          {mode === "add_account"
            ? "Add Account"
            : `Update Balance — ${accountToUpdate?.name}`}
        </h2>

        <form onSubmit={handleSubmit} className="space-y-4">
          {mode === "add_account" && (
            <>
              <Field label="Account Name">
                <input
                  required
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. Chase Checking"
                  className={inputClass}
                />
              </Field>
              <Field label="Institution">
                <input
                  required
                  value={institution}
                  onChange={(e) => setInstitution(e.target.value)}
                  placeholder="e.g. Chase Bank"
                  className={inputClass}
                />
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Type">
                  <select
                    value={accountType}
                    onChange={(e) =>
                      setAccountType(e.target.value as AccountTypeId)
                    }
                    className={inputClass}
                  >
                    {ACCOUNT_TYPES.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.label}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Currency">
                  <select
                    value={currency}
                    onChange={(e) => setCurrency(e.target.value as Currency)}
                    className={inputClass}
                  >
                    <option value="USD">USD</option>
                    <option value="CAD">CAD</option>
                  </select>
                </Field>
              </div>
              <Field label="Jurisdiction">
                <div className="flex gap-3">
                  {(["US", "CA"] as Jurisdiction[]).map((j) => (
                    <label
                      key={j}
                      className="flex items-center gap-2 cursor-pointer"
                    >
                      <input
                        type="radio"
                        name="jurisdiction"
                        value={j}
                        checked={jurisdiction === j}
                        onChange={() => setJurisdiction(j)}
                        className="accent-indigo-500"
                      />
                      <span className="text-sm text-slate-300">{j}</span>
                    </label>
                  ))}
                </div>
              </Field>
              <Field label="Notes (optional)">
                <input
                  value={notes}
                  onChange={(e) => setNotes(e.target.value)}
                  placeholder="e.g. TFSA contribution room left"
                  className={inputClass}
                />
              </Field>
            </>
          )}

          {mode === "update_balance" && (
            <>
              <Field label={`Balance (${accountToUpdate?.currency ?? ""})`}>
                <input
                  required
                  type="number"
                  step="0.01"
                  value={balance}
                  onChange={(e) => setBalance(e.target.value)}
                  placeholder="0.00"
                  className={inputClass}
                />
              </Field>
              <Field label="As of Date">
                <input
                  required
                  type="date"
                  value={snapshotDate}
                  onChange={(e) => setSnapshotDate(e.target.value)}
                  className={inputClass}
                />
              </Field>
            </>
          )}

          {error && (
            <p className="text-sm text-red-400 bg-red-900/20 rounded-lg px-3 py-2">
              {error}
            </p>
          )}

          <div className="flex gap-3 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 rounded-lg border border-slate-600 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting}
              className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
            >
              {submitting ? "Saving…" : "Save"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500";

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1.5 block text-xs font-medium text-slate-400 uppercase tracking-wider">
        {label}
      </label>
      {children}
    </div>
  );
}
