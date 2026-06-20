import { useRef, useState } from "react";
import type {
  AccountTypeId,
  Currency,
  ImportAccountInput,
  ImportPayload,
  ImportSummary,
  Jurisdiction,
} from "../types/finance";
import { importData } from "../hooks/useFinanceApi";

interface Props {
  isOpen: boolean;
  onClose: () => void;
  onImported: () => void;
}

const SAMPLE_JSON = `{
  "accounts": [
    {
      "name": "Chequing",
      "institution": "Scotiabank",
      "account_type": "chequing",
      "currency": "CAD",
      "jurisdiction": "CA",
      "snapshots": [
        { "snapshot_date": "2025-01-01", "balance": 4200.5 },
        { "snapshot_date": "2025-02-01", "balance": 5100 }
      ]
    }
  ]
}`;

export default function ImportModal({ isOpen, onClose, onImported }: Props) {
  const [jsonText, setJsonText] = useState("");
  const [csv, setCsv] = useState<{ name: string; text: string } | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  if (!isOpen) return null;

  const handleFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setError(null);
    const text = await file.text();
    if (file.name.toLowerCase().endsWith(".csv")) {
      setCsv({ name: file.name, text });
    } else {
      setJsonText(text);
      setCsv(null);
    }
    if (fileRef.current) fileRef.current.value = "";
  };

  const handleImport = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const payload = csv
        ? parseCsvToPayload(csv.text)
        : parseJsonToPayload(jsonText);
      const result = await importData(payload);
      setSummary(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const handleDone = () => {
    onImported();
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="w-full max-w-lg rounded-2xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
        <h2 className="text-lg font-semibold text-white mb-1">Import data</h2>
        <p className="text-xs text-slate-400 mb-5">
          Seed accounts and historical balances from JSON or CSV. Re-importing the
          same data won't create duplicates.
        </p>

        {summary ? (
          <ImportResult summary={summary} />
        ) : (
          <div className="space-y-4">
            {csv ? (
              <div className="flex items-center justify-between rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-slate-300">
                <span>
                  📄 {csv.name}{" "}
                  <span className="text-slate-500">(CSV will be imported)</span>
                </span>
                <button
                  type="button"
                  onClick={() => setCsv(null)}
                  className="text-slate-400 hover:text-white"
                  title="Remove file"
                >
                  ✕
                </button>
              </div>
            ) : (
              <div>
                <label className="mb-1.5 block text-xs font-medium text-slate-400 uppercase tracking-wider">
                  Paste JSON
                </label>
                <textarea
                  value={jsonText}
                  onChange={(e) => setJsonText(e.target.value)}
                  placeholder={SAMPLE_JSON}
                  spellCheck={false}
                  className="h-44 w-full resize-y rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 font-mono text-xs text-slate-200 placeholder-slate-600 focus:outline-none focus:ring-2 focus:ring-indigo-500"
                />
              </div>
            )}

            <div className="flex items-center gap-3 text-xs text-slate-500">
              <span className="h-px flex-1 bg-slate-700" />
              or
              <span className="h-px flex-1 bg-slate-700" />
            </div>

            <div>
              <input
                ref={fileRef}
                type="file"
                accept=".json,.csv"
                onChange={handleFile}
                className="block w-full text-sm text-slate-400 file:mr-3 file:rounded-lg file:border-0 file:bg-slate-700 file:px-3 file:py-1.5 file:text-sm file:font-medium file:text-slate-200 hover:file:bg-slate-600"
              />
              <p className="mt-2 text-[11px] text-slate-500">
                CSV columns: institution, name, account_type, currency,
                jurisdiction, snapshot_date, balance
              </p>
            </div>

            {error && (
              <p className="rounded-lg bg-red-900/20 px-3 py-2 text-sm text-red-400">
                {error}
              </p>
            )}
          </div>
        )}

        <div className="flex gap-3 pt-5">
          {summary ? (
            <button
              type="button"
              onClick={handleDone}
              className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 transition-colors"
            >
              Done
            </button>
          ) : (
            <>
              <button
                type="button"
                onClick={onClose}
                className="flex-1 rounded-lg border border-slate-600 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleImport}
                disabled={submitting || (!csv && !jsonText.trim())}
                className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {submitting ? "Importing…" : "Import"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function ImportResult({ summary }: { summary: ImportSummary }) {
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-3 gap-3">
        <Stat label="Created" value={summary.accounts_created} />
        <Stat label="Matched" value={summary.accounts_matched} />
        <Stat label="Snapshots" value={summary.snapshots_imported} />
      </div>
      {summary.errors.length > 0 && (
        <div className="rounded-lg bg-amber-900/20 border border-amber-700/40 px-3 py-2 text-xs text-amber-300">
          <p className="mb-1 font-semibold">
            {summary.errors.length} row(s) skipped:
          </p>
          <ul className="list-disc list-inside space-y-0.5">
            {summary.errors.slice(0, 8).map((e, i) => (
              <li key={i}>{e}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border border-slate-700 bg-slate-800/60 px-3 py-2 text-center">
      <p className="text-xl font-bold text-white">{value}</p>
      <p className="text-[11px] uppercase tracking-wider text-slate-400">
        {label}
      </p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

function parseJsonToPayload(text: string): ImportPayload {
  if (!text.trim()) throw new Error("Paste some JSON or choose a file first.");
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    throw new Error("That isn't valid JSON.");
  }
  const accounts = Array.isArray(data)
    ? data
    : (data as { accounts?: unknown }).accounts;
  if (!Array.isArray(accounts)) {
    throw new Error('Expected an object with an "accounts" array.');
  }
  return {
    accounts: accounts.map((a) => {
      const acc = a as Partial<ImportAccountInput>;
      return {
        ...acc,
        snapshots: Array.isArray(acc.snapshots) ? acc.snapshots : [],
      } as ImportAccountInput;
    }),
  };
}

function parseCsvToPayload(text: string): ImportPayload {
  const rows = parseCsvRows(text);
  if (rows.length < 2) {
    throw new Error("CSV needs a header row and at least one data row.");
  }
  const header = rows[0].map((h) => h.trim().toLowerCase());
  const col = (name: string) => header.indexOf(name);
  const iInst = col("institution");
  const iName = col("name");
  if (iInst < 0 || iName < 0) {
    throw new Error("CSV must include 'institution' and 'name' columns.");
  }
  const iType = col("account_type");
  const iCur = col("currency");
  const iJur = col("jurisdiction");
  const iDate = col("snapshot_date");
  const iBal = col("balance");
  const iNotes = col("notes");

  const byKey = new Map<string, ImportAccountInput>();
  for (let r = 1; r < rows.length; r++) {
    const cells = rows[r];
    const institution = (cells[iInst] ?? "").trim();
    const name = (cells[iName] ?? "").trim();
    if (!institution || !name) continue;

    const key = `${institution.toLowerCase()}||${name.toLowerCase()}`;
    let acc = byKey.get(key);
    if (!acc) {
      acc = {
        name,
        institution,
        account_type:
          (iType >= 0 ? ((cells[iType] ?? "").trim() as AccountTypeId) : "") ||
          ("other" as AccountTypeId),
        currency:
          (iCur >= 0
            ? ((cells[iCur] ?? "").trim().toUpperCase() as Currency)
            : "") || ("USD" as Currency),
        jurisdiction:
          (iJur >= 0
            ? ((cells[iJur] ?? "").trim().toUpperCase() as Jurisdiction)
            : "") || ("US" as Jurisdiction),
        notes: iNotes >= 0 ? (cells[iNotes] ?? "").trim() || null : null,
        snapshots: [],
      };
      byKey.set(key, acc);
    }

    if (iDate >= 0 && iBal >= 0) {
      const dateVal = (cells[iDate] ?? "").trim();
      const balVal = (cells[iBal] ?? "").trim();
      if (dateVal && balVal) {
        const balance = Number(balVal.replace(/[$,]/g, ""));
        if (!Number.isNaN(balance)) {
          acc.snapshots.push({ snapshot_date: dateVal, balance });
        }
      }
    }
  }

  const accounts = [...byKey.values()];
  if (accounts.length === 0) throw new Error("No valid rows found in the CSV.");
  return { accounts };
}

/** Minimal RFC-4180-ish CSV tokenizer (handles quoted fields and embedded commas). */
function parseCsvRows(text: string): string[][] {
  const rows: string[][] = [];
  let field = "";
  let row: string[] = [];
  let inQuotes = false;

  const pushRow = () => {
    row.push(field);
    field = "";
    if (row.some((v) => v.trim() !== "")) rows.push(row);
    row = [];
  };

  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inQuotes) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          inQuotes = false;
        }
      } else {
        field += c;
      }
    } else if (c === '"') {
      inQuotes = true;
    } else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      pushRow();
    } else {
      field += c;
    }
  }
  if (field.length > 0 || row.length > 0) pushRow();
  return rows;
}
