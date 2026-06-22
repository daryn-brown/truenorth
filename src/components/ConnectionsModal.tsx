import { useCallback, useEffect, useState } from "react";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import type { SnapTradeStatus, SnapTradeSyncSummary } from "../types/finance";
import {
  snaptradeDisconnect,
  snaptradeGetLoginLink,
  snaptradeGetStatus,
  snaptradeSaveCredentials,
  snaptradeSync,
} from "../hooks/useFinanceApi";

interface Props {
  isOpen: boolean;
  onClose: () => void;
  /** Called after a successful sync or disconnect so the dashboard can reload. */
  onChanged: () => void;
}

const SNAPTRADE_DASHBOARD = "https://dashboard.snaptrade.com";

const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:ring-2 focus:ring-indigo-500";

export default function ConnectionsModal({ isOpen, onClose, onChanged }: Props) {
  const [status, setStatus] = useState<SnapTradeStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [consumerKey, setConsumerKey] = useState("");
  const [busy, setBusy] = useState<null | "save" | "connect" | "sync" | "disconnect">(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [summary, setSummary] = useState<SnapTradeSyncSummary | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await snaptradeGetStatus());
    } catch (err) {
      setError(messageOf(err));
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    setInfo(null);
    setSummary(null);
    void refreshStatus();
  }, [isOpen, refreshStatus]);

  if (!isOpen) return null;

  const handleSave = async () => {
    setBusy("save");
    setError(null);
    setInfo(null);
    try {
      const next = await snaptradeSaveCredentials(clientId, consumerKey);
      setStatus(next);
      setConsumerKey("");
      setInfo("API key saved and verified.");
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const handleConnect = async () => {
    setBusy("connect");
    setError(null);
    setInfo(null);
    try {
      const url = await snaptradeGetLoginLink();
      await openUrl(url);
      await refreshStatus();
      setInfo(
        "A secure SnapTrade window opened in your browser. Authorize your brokerage there, then come back and click “Sync now”.",
      );
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const handleSync = async () => {
    setBusy("sync");
    setError(null);
    setInfo(null);
    setSummary(null);
    try {
      const result = await snaptradeSync();
      setSummary(result);
      await refreshStatus();
      onChanged();
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const handleDisconnect = async () => {
    if (
      !confirm(
        "Disconnect your brokerage? Connected accounts will be hidden and synced balances stop updating. Your API key stays saved so you can reconnect.",
      )
    ) {
      return;
    }
    setBusy("disconnect");
    setError(null);
    setInfo(null);
    try {
      const next = await snaptradeDisconnect();
      setStatus(next);
      setSummary(null);
      onChanged();
      setInfo("Brokerage disconnected.");
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const hasCredentials = status?.has_credentials ?? false;
  const isConnected = status?.is_connected ?? false;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="w-full max-w-lg rounded-2xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
        <h2 className="mb-1 text-lg font-semibold text-white">Connect a brokerage</h2>
        <p className="mb-5 text-xs text-slate-400">
          Sync real balances from Robinhood, Questrade, Wealthsimple and more via{" "}
          <button
            type="button"
            onClick={() => void openUrl(SNAPTRADE_DASHBOARD)}
            className="text-indigo-400 underline hover:text-indigo-300"
          >
            SnapTrade
          </button>
          . TrueNorth requests <span className="font-semibold text-slate-300">read-only</span>{" "}
          access — it can never place trades. Your keys are stored in your OS keychain, never on
          disk.
        </p>

        {/* Step 1 — API credentials */}
        <Section
          step={1}
          title="SnapTrade API key"
          done={hasCredentials}
        >
          {hasCredentials ? (
            <div className="flex items-center justify-between gap-3 text-sm text-slate-300">
              <span className="truncate">
                Saved
                {status?.client_id ? (
                  <span className="text-slate-500"> · {status.client_id}</span>
                ) : null}
              </span>
              <button
                type="button"
                onClick={() => setStatus((s) => (s ? { ...s, has_credentials: false } : s))}
                className="shrink-0 text-xs text-slate-400 underline hover:text-slate-200"
              >
                Change
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <p className="text-xs text-slate-500">
                Create a free developer account, then copy your Client ID and Consumer Key from
                the{" "}
                <button
                  type="button"
                  onClick={() => void openUrl(SNAPTRADE_DASHBOARD)}
                  className="text-indigo-400 underline hover:text-indigo-300"
                >
                  SnapTrade dashboard
                </button>
                .
              </p>
              <input
                value={clientId}
                onChange={(e) => setClientId(e.target.value)}
                placeholder="Client ID"
                spellCheck={false}
                className={inputClass}
              />
              <input
                value={consumerKey}
                onChange={(e) => setConsumerKey(e.target.value)}
                placeholder="Consumer Key"
                type="password"
                spellCheck={false}
                className={inputClass}
              />
              <button
                type="button"
                onClick={handleSave}
                disabled={busy !== null || !clientId.trim() || !consumerKey.trim()}
                className="w-full rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {busy === "save" ? "Verifying…" : "Save & verify"}
              </button>
            </div>
          )}
        </Section>

        {/* Step 2 — Authorize a brokerage */}
        <Section
          step={2}
          title="Authorize your brokerage"
          done={isConnected}
          disabled={!hasCredentials}
        >
          <p className="mb-3 text-xs text-slate-500">
            Opens SnapTrade's secure connection portal in your browser to link an institution.
          </p>
          <button
            type="button"
            onClick={handleConnect}
            disabled={busy !== null || !hasCredentials}
            className="w-full rounded-lg border border-slate-600 py-2 text-sm font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50 transition-colors"
          >
            {busy === "connect"
              ? "Opening…"
              : isConnected
                ? "Connect another brokerage"
                : "Connect a brokerage"}
          </button>
        </Section>

        {/* Step 3 — Sync */}
        <Section
          step={3}
          title="Sync balances"
          disabled={!isConnected}
          last
        >
          <div className="flex items-center justify-between gap-3">
            <p className="text-xs text-slate-500">
              {status?.account_count
                ? `${status.account_count} account${status.account_count === 1 ? "" : "s"} connected`
                : "No accounts synced yet"}
              {status?.last_synced_at ? ` · last synced ${formatStamp(status.last_synced_at)}` : ""}
            </p>
            <button
              type="button"
              onClick={handleSync}
              disabled={busy !== null || !isConnected}
              className="shrink-0 rounded-lg bg-indigo-600 px-4 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
            >
              {busy === "sync" ? "Syncing…" : "Sync now"}
            </button>
          </div>

          {summary && (
            <p className="mt-3 rounded-lg bg-emerald-900/20 border border-emerald-700/40 px-3 py-2 text-xs text-emerald-300">
              Synced {summary.accounts_synced} account
              {summary.accounts_synced === 1 ? "" : "s"} and {summary.holdings_synced} holding
              {summary.holdings_synced === 1 ? "" : "s"}. Net worth is up to date.
            </p>
          )}
        </Section>

        {info && (
          <p className="mt-4 rounded-lg bg-slate-800 border border-slate-700 px-3 py-2 text-xs text-slate-300">
            {info}
          </p>
        )}
        {error && (
          <p className="mt-4 rounded-lg bg-red-900/20 px-3 py-2 text-sm text-red-400">{error}</p>
        )}

        <div className="flex items-center justify-between pt-5">
          {isConnected ? (
            <button
              type="button"
              onClick={handleDisconnect}
              disabled={busy !== null}
              className="text-xs font-medium text-red-400 hover:text-red-300 disabled:opacity-50"
            >
              {busy === "disconnect" ? "Disconnecting…" : "Disconnect brokerage"}
            </button>
          ) : (
            <span />
          )}
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

function Section({
  step,
  title,
  children,
  done,
  disabled,
  last,
}: {
  step: number;
  title: string;
  children: React.ReactNode;
  done?: boolean;
  disabled?: boolean;
  last?: boolean;
}) {
  return (
    <div className={`${disabled ? "opacity-50" : ""} ${last ? "" : "mb-4 border-b border-slate-800 pb-4"}`}>
      <div className="mb-2 flex items-center gap-2">
        <span
          className={`flex h-5 w-5 items-center justify-center rounded-full text-[11px] font-bold ${
            done
              ? "bg-emerald-600 text-white"
              : "border border-slate-600 text-slate-400"
          }`}
        >
          {done ? "✓" : step}
        </span>
        <h3 className="text-sm font-semibold text-slate-200">{title}</h3>
      </div>
      <div className="pl-7">{children}</div>
    </div>
  );
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function formatStamp(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
}
