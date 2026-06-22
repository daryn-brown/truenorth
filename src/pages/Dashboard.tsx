import { useCallback, useEffect, useState } from "react";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  Currency,
  NetWorth,
  NetWorthDelta,
  NetWorthHistoryPoint,
} from "../types/finance";
import {
  addAccount,
  addBalanceSnapshot,
  deleteAccount,
  getNetWorth,
  getNetWorthDelta,
  getNetWorthHistory,
  listAccounts,
  refreshFxRates,
} from "../hooks/useFinanceApi";
import NetWorthCard from "../components/NetWorthCard";
import AccountList from "../components/AccountList";
import AccountModal from "../components/AccountModal";
import ImportModal from "../components/ImportModal";
import ConnectionsModal from "../components/ConnectionsModal";
import NetWorthChart from "../components/NetWorthChart";

type ModalState =
  | { open: false }
  | { open: true; mode: "add_account" }
  | { open: true; mode: "update_balance"; account: Account };

export default function Dashboard() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [netWorth, setNetWorth] = useState<NetWorth | null>(null);
  const [delta, setDelta] = useState<NetWorthDelta | null>(null);
  const [history, setHistory] = useState<NetWorthHistoryPoint[]>([]);
  const [homeCurrency, setHomeCurrency] = useState<Currency>("CAD");
  const [loading, setLoading] = useState(true);
  const [modal, setModal] = useState<ModalState>({ open: false });
  const [importOpen, setImportOpen] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  const [refreshingFx, setRefreshingFx] = useState(false);
  const [fxError, setFxError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [accs, nw, nwDelta, hist] = await Promise.all([
        listAccounts(),
        getNetWorth(),
        getNetWorthDelta(),
        getNetWorthHistory(),
      ]);
      setAccounts(accs);
      setNetWorth(nw);
      setDelta(nwDelta);
      setHistory(hist);
    } catch (err) {
      console.error("Failed to load dashboard data:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleAddAccount = async (payload: AddAccountPayload) => {
    await addAccount(payload);
    await load();
  };

  const handleUpdateBalance = async (payload: AddBalanceSnapshotPayload) => {
    await addBalanceSnapshot(payload);
    await load();
  };

  const handleDeleteAccount = async (id: number) => {
    if (!confirm("Delete this account and all its snapshots?")) return;
    await deleteAccount(id);
    await load();
  };

  const handleRefreshFx = async () => {
    setRefreshingFx(true);
    setFxError(null);
    try {
      await refreshFxRates();
      await load();
    } catch (err) {
      setFxError(String(err));
    } finally {
      setRefreshingFx(false);
    }
  };

  const handleConnectorChanged = useCallback(async () => {
    // A sync may have added accounts in new currencies (e.g. JMD) — refresh FX so they
    // convert into the totals. A rate-fetch failure shouldn't hide the freshly synced balances.
    try {
      await refreshFxRates();
    } catch (err) {
      console.error("FX refresh after connector sync failed:", err);
    }
    await load();
  }, [load]);

  // Net-worth-over-time series in the selected home currency.
  const chartData = history.map((point) => ({
    date: point.date,
    value: homeCurrency === "CAD" ? point.total_cad : point.total_usd,
  }));

  return (
    <div className="min-h-screen bg-slate-950 text-white">
      {/* Header */}
      <header className="border-b border-slate-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <span className="text-xl font-bold text-white tracking-tight">
            🧭 TrueNorth
          </span>
          <span className="rounded-full bg-indigo-900/50 border border-indigo-700/50 px-2 py-0.5 text-[11px] font-semibold text-indigo-300 uppercase tracking-wider">
            Phase 3
          </span>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setConnectOpen(true)}
            title="Connect brokerages (SnapTrade) and banks (SimpleFIN) to sync real balances"
            className="flex items-center gap-1.5 rounded-lg border border-indigo-700/60 bg-indigo-900/30 px-3 py-1.5 text-xs text-indigo-200 hover:bg-indigo-900/60 transition-colors"
          >
            🔗 Connect
          </button>
          <button
            onClick={() => setImportOpen(true)}
            title="Import accounts and balance history from JSON or CSV"
            className="flex items-center gap-1.5 rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-400 hover:bg-slate-800 transition-colors"
          >
            ⬆️ Import
          </button>
          <button
            onClick={handleRefreshFx}
            disabled={refreshingFx}
            title="Refresh USD/CAD exchange rate from Yahoo Finance"
            className="flex items-center gap-1.5 rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-400 hover:bg-slate-800 disabled:opacity-50 transition-colors"
          >
            {refreshingFx ? "Refreshing…" : "🔄 Refresh FX"}
          </button>
        </div>
      </header>

      <main className="mx-auto max-w-3xl px-6 py-8 space-y-6">
        {fxError && (
          <div className="rounded-lg bg-red-900/20 border border-red-700/50 px-4 py-3 text-sm text-red-400">
            FX refresh failed: {fxError}
          </div>
        )}

        {/* Net worth summary */}
        <NetWorthCard
          netWorth={netWorth}
          delta={delta}
          homeCurrency={homeCurrency}
          onToggleCurrency={() =>
            setHomeCurrency((c) => (c === "CAD" ? "USD" : "CAD"))
          }
          loading={loading}
        />

        {/* Net worth chart */}
        <NetWorthChart data={chartData} currency={homeCurrency} />

        {/* Account list */}
        <AccountList
          accounts={accounts}
          netWorthBreakdown={netWorth?.accounts ?? []}
          onAddAccount={() => setModal({ open: true, mode: "add_account" })}
          onDeleteAccount={handleDeleteAccount}
          onUpdateBalance={(account) =>
            setModal({ open: true, mode: "update_balance", account })
          }
        />

        {accounts.length === 0 && !loading && (
          <p className="text-center text-xs text-slate-600 pt-2">
            All data is stored locally and encrypted. Nothing leaves your device.
          </p>
        )}
      </main>

      {/* Modals */}
      {modal.open && modal.mode === "add_account" && (
        <AccountModal
          isOpen
          mode="add_account"
          onClose={() => setModal({ open: false })}
          onAddAccount={handleAddAccount}
          onUpdateBalance={handleUpdateBalance}
        />
      )}
      {modal.open && modal.mode === "update_balance" && (
        <AccountModal
          isOpen
          mode="update_balance"
          accountToUpdate={modal.account}
          onClose={() => setModal({ open: false })}
          onAddAccount={handleAddAccount}
          onUpdateBalance={handleUpdateBalance}
        />
      )}

      <ImportModal
        isOpen={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={load}
      />

      <ConnectionsModal
        isOpen={connectOpen}
        onClose={() => setConnectOpen(false)}
        onChanged={handleConnectorChanged}
      />
    </div>
  );
}
