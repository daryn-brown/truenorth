import { useCallback, useEffect, useId, useState } from "react";
import { discoverSyncAccounts, saveSyncAccountChoices } from "../hooks/useFinanceApi";
import { ACCOUNT_TYPE_LABELS, CONNECTOR_LABELS } from "../shared/accountLabels";
import type {
  AccountSyncChoice, AccountSyncReview, DiscoveredSyncAccount, SyncProvider,
} from "../types/finance";
import ConfirmationDialog from "./ConfirmationDialog";

interface Props {
  provider: SyncProvider;
  onCancel: () => void;
  onSaved: () => Promise<void>;
  onBusyChange: (busy: boolean) => void;
}

interface Draft {
  action: "unreviewed" | "create" | "link" | "sync" | "ignore";
  accountId: string;
}

function initialDrafts(review: AccountSyncReview): Record<string, Draft> {
  return Object.fromEntries(review.accounts.map((a) => [
    a.remote_id, { action: a.selection, accountId: "" },
  ]));
}

const identity = (account: DiscoveredSyncAccount) =>
  `${account.name}${account.masked_number ? ` (${account.masked_number})` : ""}`;
const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-50";

export default function AccountSyncSelection({ provider, onCancel, onSaved, onBusyChange }: Props) {
  const [review, setReview] = useState<AccountSyncReview | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [reload, setReload] = useState(0);
  const id = useId();
  const busy = loading || saving;

  useEffect(() => {
    onBusyChange(busy);
    return () => onBusyChange(false);
  }, [busy, onBusyChange]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setReview(null);
    void discoverSyncAccounts(provider).then(
      (result) => {
        if (!cancelled) {
          setReview(result);
          setDrafts(initialDrafts(result));
          setLoading(false);
        }
      },
      (err: unknown) => {
        if (!cancelled) {
          setError(String(err));
          setLoading(false);
        }
      },
    );
    return () => { cancelled = true; };
  }, [provider, reload]);

  const choices: AccountSyncChoice[] = [];
  let incomplete = false;
  for (const account of review?.accounts ?? []) {
    const draft = drafts[account.remote_id];
    if (!draft || draft.action === "unreviewed" || draft.action === account.selection) continue;
    if (draft.action === "link") {
      if (!draft.accountId) incomplete = true;
      else choices.push({ remote_id: account.remote_id, action: "link", account_id: Number(draft.accountId) });
    } else {
      choices.push({ remote_id: account.remote_id, action: draft.action });
    }
  }
  const handoffs = choices.filter((c) => c.action === "link" || c.action === "sync");
  const assignedTargets = new Set(choices.flatMap((c) => c.action === "link" ? [c.account_id] : []));

  const changeDraft = useCallback((remoteId: string, change: Partial<Draft>) => {
    setDrafts((current) => ({
      ...current, [remoteId]: { ...current[remoteId], ...change },
    }));
    setError(null);
  }, []);

  const save = async () => {
    if (!review) return;
    setSaving(true);
    setError(null);
    let saved = false;
    try {
      const result = await saveSyncAccountChoices(provider, { revision: review.revision, choices });
      saved = true;
      setReview(result);
      setDrafts(initialDrafts(result));
      setConfirming(false);
      await onSaved();
    } catch (err) {
      setError(saved ? `Choices were saved, but the connection status could not refresh: ${String(err)}` : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section aria-labelledby={`${id}-title`} aria-busy={busy}>
      <h3 id={`${id}-title`} className="text-base font-semibold text-white">Choose accounts to sync</h3>
      <p className="mt-2 text-xs text-slate-400">
        {CONNECTOR_LABELS[provider]} may return accounts from every connected institution.
        New accounts stay excluded until you choose Create new or Link to existing.
        Matching names, balances, or number endings do not prove two accounts are the same.
      </p>
      <p className="mt-2 text-xs text-slate-400">
        Ignore hides an imported account from totals and charts but keeps its stored history.
        Imported accounts cannot be reassigned or merged; Ignore is the safe way to hide an
        existing duplicate without changing the original.
      </p>
      {loading && <p role="status" className="mt-4 text-sm text-slate-300">Discovering accounts...</p>}
      {review && (
        <div className="mt-4 space-y-3">
          {review.accounts.length === 0 && (
            <p role="status" className="text-sm text-slate-300">
              No accounts were returned. Finish connecting the institution at {CONNECTOR_LABELS[provider]}, then reload.
            </p>
          )}
          {review.accounts.map((account, index) => {
            const draft = drafts[account.remote_id];
            const local = review.local_accounts.find((a) => a.id === account.local_account_id);
            const target = review.local_accounts.find((a) => String(a.id) === draft?.accountId);
            const transferred = local && local.connector_kind !== provider;
            const labelId = `${id}-${index}`;
            return (
              <fieldset key={account.remote_id} className="rounded-xl border border-slate-700 p-3" disabled={busy}>
                <legend className="px-1 text-sm font-semibold text-slate-200">{identity(account)}</legend>
                <p className="text-xs text-slate-400">
                  {account.institution} · {ACCOUNT_TYPE_LABELS[account.account_type]} · {account.currency || "Currency not reported"}
                </p>
                <p className={`mt-2 text-xs ${account.selection === "sync" ? "text-emerald-300" : "text-amber-300"}`}>
                  {account.selection === "sync"
                    ? `Already syncing to ${local?.name}`
                    : account.selection === "ignore"
                      ? transferred
                        ? `Ignored here; ${local.name} now uses ${CONNECTOR_LABELS[local.connector_kind]}`
                        : "Ignored - will not sync"
                      : "New - not selected"}
                </p>
                {local && (
                  <p className="mt-1 text-xs text-slate-400">
                    History stays with {local.name} (local account #{local.id}).
                    {transferred ? " Resuming switches its source back." : " Resume uses this same row, not a new copy."}
                  </p>
                )}
                <label htmlFor={`${labelId}-action`} className="mb-1 mt-3 block text-xs text-slate-300">
                  Sync choice for {identity(account)}
                </label>
                <select
                  id={`${labelId}-action`}
                  className={inputClass}
                  value={draft?.action ?? account.selection}
                  onChange={(event) => {
                    const action = event.target.value;
                    if (action === "unreviewed" || action === "create" || action === "link" || action === "sync" || action === "ignore") {
                      changeDraft(account.remote_id, { action, accountId: "" });
                    }
                  }}
                >
                  {account.selection === "unreviewed" && <option value="unreviewed">Not selected - review later</option>}
                  {local ? (
                    <option value="sync" disabled={!account.can_resume}>
                      {account.selection === "sync" ? "Keep syncing" : "Resume syncing to the same account"}
                    </option>
                  ) : (
                    <>
                      <option value="create" disabled={Boolean(account.unavailable_reason)}>Create new account</option>
                      <option value="link" disabled={account.linkable_account_ids.length === 0}>Link to existing account</option>
                    </>
                  )}
                  <option value="ignore">{local && !transferred ? "Ignore (hide this imported account)" : "Ignore (do not sync)"}</option>
                </select>
                {account.unavailable_reason && (
                  <p className="mt-2 text-xs text-amber-300">{account.unavailable_reason}</p>
                )}
                {draft?.action === "link" && (
                  <>
                    <label htmlFor={`${labelId}-target`} className="mb-1 mt-3 block text-xs text-slate-300">
                      Existing account for {identity(account)}
                    </label>
                    <select
                      id={`${labelId}-target`}
                      className={inputClass}
                      value={draft.accountId}
                      onChange={(event) => changeDraft(account.remote_id, { accountId: event.target.value })}
                    >
                      <option value="">Choose the same real account...</option>
                      {review.local_accounts.filter((a) => account.linkable_account_ids.includes(a.id)).map((a) => (
                        <option
                          key={a.id}
                          value={a.id}
                          disabled={assignedTargets.has(a.id) && draft.accountId !== String(a.id)}
                        >
                          {a.name} - {a.institution} - {a.currency} - {CONNECTOR_LABELS[a.connector_kind]} (#{a.id})
                        </option>
                      ))}
                    </select>
                    {target && (
                      <p className="mt-2 text-xs text-amber-300">
                        Switches {target.name} from {CONNECTOR_LABELS[target.connector_kind]} to {CONNECTOR_LABELS[provider]}.
                        Its history and labels stay; the old provider source is excluded from future syncs.
                      </p>
                    )}
                  </>
                )}
                {!local && !account.unavailable_reason && account.linkable_account_ids.length === 0 && (
                  <p className="mt-2 text-xs text-slate-400">
                    No compatible existing account. Linking requires the same currency and account type,
                    and cannot replace a different account ID from this provider.
                  </p>
                )}
              </fieldset>
            );
          })}
          {review.warnings.map((warning, index) => (
            <p key={index} role="status" className="text-xs text-amber-300">{warning}</p>
          ))}
        </div>
      )}
      {error && !confirming && <p role="alert" className="mt-3 text-sm text-red-400">{error}</p>}
      <div className="mt-4 flex flex-wrap justify-end gap-2">
        <button type="button" disabled={busy} onClick={onCancel} className="rounded-lg border border-slate-600 px-3 py-2 text-sm text-slate-300 disabled:opacity-50">
          Back
        </button>
        <button type="button" disabled={busy} onClick={() => setReload((n) => n + 1)} className="rounded-lg border border-slate-600 px-3 py-2 text-sm text-slate-300 disabled:opacity-50">
          Reload accounts
        </button>
        <button
          type="button"
          disabled={busy || !review || choices.length === 0 || incomplete}
          onClick={() => handoffs.length > 0 ? setConfirming(true) : void save()}
          className="rounded-lg bg-indigo-600 px-3 py-2 text-sm font-semibold text-white disabled:opacity-50"
        >
          {saving ? "Saving..." : "Save choices"}
        </button>
      </div>
      {confirming && review && (
        <ConfirmationDialog
          title="Confirm account selection"
          confirmLabel="Confirm and save"
          busy={saving}
          error={error}
          onCancel={() => setConfirming(false)}
          onConfirm={() => void save()}
        >
          <p>Confirm that each link is the same real account, not just a similar name or balance.</p>
          {handoffs.map((choice) => {
            const remote = review.accounts.find((a) => a.remote_id === choice.remote_id);
            const targetId = choice.action === "link" ? choice.account_id : remote?.local_account_id;
            const target = review.local_accounts.find((a) => a.id === targetId);
            return (
              <p key={choice.remote_id}>
                <strong>{remote && identity(remote)}</strong> will sync to <strong>{target?.name}</strong>
                {target ? ` (local account #${target.id})` : ""} via {CONNECTOR_LABELS[provider]}.
                {target && target.connector_kind !== provider
                  ? ` This switches its source from ${CONNECTOR_LABELS[target.connector_kind]} and excludes the old source.`
                  : " This restores the hidden account and includes it in totals again."}
              </p>
            );
          })}
          <p>Existing account IDs, history, transactions, and user labels are retained. Accounts are not merged.</p>
        </ConfirmationDialog>
      )}
    </section>
  );
}
