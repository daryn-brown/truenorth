import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  CashflowSummary,
  Currency,
  DividendSummary,
  FireInputs,
  FirePlan,
  GoalProgress,
  NetWorth,
  NetWorthDelta,
  NetWorthHistoryPoint,
  SeattleAssumptions,
  SeattleProjection,
  ProgressMetrics,
  ProgressInputs,
  MacWidgetSettings,
  SimpleFinStatus,
} from "../types/finance";
import {
  addAccount,
  addBalanceSnapshot,
  deleteAccount,
  getCashflowSummary,
  getDividendSummary,
  getFirePlan,
  getGoalProgress,
  getNetWorth,
  getNetWorthDelta,
  getNetWorthHistory,
  getMacWidgetSettings,
  refreshMacWidget,
  getSeattleProjection,
  getProgressMetrics,
  listAccounts,
  refreshFxRates,
  refreshFxRatesIfStale,
  setFireInputs,
  setSeattleAssumptions,
  setProgressInputs,
  updateAccountCurrency,
  simplefinGetStatus,
  simplefinSync,
  snaptradeGetStatus,
  snaptradeSync,
  tellerGetStatus,
  tellerSync,
  questradeGetStatus,
  questradeSync,
} from "../hooks/useFinanceApi";
import NetWorthCard from "../components/NetWorthCard";
import GoalCountdownCard from "../components/GoalCountdownCard";
import FirePlannerCard from "../components/FirePlannerCard";
import ProgressCard from "../components/ProgressCard";
import SeattleSimulatorCard from "../components/SeattleSimulatorCard";
import CashflowCard from "../components/CashflowCard";
import DividendIncomeCard from "../components/DividendIncomeCard";
import AccountList from "../components/AccountList";
import AccountModal from "../components/AccountModal";
import ImportModal from "../components/ImportModal";
import ConnectionsModal from "../components/ConnectionsModal";
import NetWorthChart from "../components/NetWorthChart";
import DesktopIcon from "../apps/desktop/DesktopIcon";
import BrandMark from "../shared/BrandMark";
import MacWidgetModal from "../components/MacWidgetModal";

type ModalState =
  | { open: false }
  | { open: true; mode: "add_account" }
  | { open: true; mode: "update_balance"; account: Account }
  | { open: true; mode: "edit_currency"; account: Account };

export default function Dashboard({
  onCheckForUpdates,
  checkingUpdate = false,
  onToggleAdvisor,
}: {
  onCheckForUpdates?: () => void;
  checkingUpdate?: boolean;
  onToggleAdvisor?: () => void;
} = {}) {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [netWorth, setNetWorth] = useState<NetWorth | null>(null);
  const [delta, setDelta] = useState<NetWorthDelta | null>(null);
  const [goal, setGoal] = useState<GoalProgress | null>(null);
  const [firePlan, setFirePlan] = useState<FirePlan | null>(null);
  const [progress, setProgress] = useState<ProgressMetrics | null>(null);
  const [projection, setProjection] = useState<SeattleProjection | null>(null);
  const [cashflow, setCashflow] = useState<CashflowSummary | null>(null);
  const [dividends, setDividends] = useState<DividendSummary | null>(null);
  const [history, setHistory] = useState<NetWorthHistoryPoint[]>([]);
  const [homeCurrency, setHomeCurrency] = useState<Currency>("CAD");
  const [loading, setLoading] = useState(true);
  const [modal, setModal] = useState<ModalState>({ open: false });
  const [importOpen, setImportOpen] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  const [refreshingFx, setRefreshingFx] = useState(false);
  const [fxError, setFxError] = useState<string | null>(null);
  const [dividendLoading, setDividendLoading] = useState(true);
  const [dividendError, setDividendError] = useState<string | null>(null);
  const [widgetSettings, setWidgetSettings] = useState<MacWidgetSettings | null>(null);
  const [widgetOpen, setWidgetOpen] = useState(false);
  const [widgetError, setWidgetError] = useState<string | null>(null);
  const [simplefinStatus, setSimplefinStatus] = useState<SimpleFinStatus | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncNotice, setSyncNotice] = useState<string | null>(null);
  const [connectProvider, setConnectProvider] = useState<"snaptrade" | "simplefin">("snaptrade");
  const syncInFlight = useRef(false);
  const dividendRequest = useRef(0);

  const refreshConnectionHealth = useCallback(async () => {
    try {
      const status = await simplefinGetStatus();
      setSimplefinStatus(status);
      setHealthError(null);
      return status;
    } catch (err) {
      setHealthError(`Could not verify SimpleFIN connection health: ${String(err)}`);
      return null;
    }
  }, []);

  const refreshWidget = useCallback(async () => {
    try {
      setWidgetSettings(await refreshMacWidget());
      setWidgetError(null);
    } catch (err) {
      console.error("Failed to refresh Mac widget:", err);
      setWidgetError(String(err));
      try {
        setWidgetSettings(await getMacWidgetSettings());
      } catch (statusError) {
        setWidgetError(`${String(err)} Could not load widget settings: ${String(statusError)}`);
      }
    }
  }, []);

  const loadDividends = useCallback(async (forceRefresh = false) => {
    const requestId = ++dividendRequest.current;
    setDividendLoading(true);
    setDividendError(null);
    try {
      const summary = await getDividendSummary(forceRefresh);
      if (requestId === dividendRequest.current) {
        setDividends(summary);
      }
    } catch (err) {
      console.error("Failed to load dividend income:", err);
      if (requestId === dividendRequest.current) {
        setDividendError(
          typeof err === "string"
            ? err
            : "Dividend research is unavailable right now.",
        );
      }
    } finally {
      if (requestId === dividendRequest.current) {
        setDividendLoading(false);
      }
    }
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      void refreshConnectionHealth();
      // Keep conversions current without a manual click: refresh FX at most once per day. This is
      // a no-op (DB check only) when today's rates are already stored, and stays best-effort so an
      // offline launch still renders with the last known rates.
      try {
        await refreshFxRatesIfStale();
      } catch (err) {
        console.error("Auto FX refresh failed:", err);
      }
      void loadDividends();
      const [accs, nw, nwDelta, goalProgress, fire, wb, proj, cf, hist] = await Promise.all([
        listAccounts(),
        getNetWorth(),
        getNetWorthDelta(),
        getGoalProgress(),
        getFirePlan(),
        getProgressMetrics(),
        getSeattleProjection(),
        getCashflowSummary(),
        getNetWorthHistory(),
      ]);
      setAccounts(accs);
      setNetWorth(nw);
      setDelta(nwDelta);
      setGoal(goalProgress);
      setFirePlan(fire);
      setProgress(wb);
      setProjection(proj);
      setCashflow(cf);
      setHistory(hist);
      setLoadError(null);
    } catch (err) {
      console.error("Failed to load dashboard data:", err);
      setLoadError(`Could not reload the dashboard. Shown figures may be out of date: ${String(err)}`);
    } finally {
      setLoading(false);
      await refreshWidget();
    }
  }, [loadDividends, refreshWidget, refreshConnectionHealth]);

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

  const handleUpdateAssumptions = useCallback(async (a: SeattleAssumptions) => {
    const updated = await setSeattleAssumptions(a);
    setProjection(updated);
  }, []);

  const handleUpdateFireInputs = useCallback(async (inputs: FireInputs) => {
    const updated = await setFireInputs(inputs);
    setFirePlan(updated);
  }, []);

  const handleUpdateProgressInputs = useCallback(async (inputs: ProgressInputs) => {
    const updated = await setProgressInputs(inputs);
    setProgress(updated);
  }, []);

  const refreshCashflow = useCallback(async () => {
    try {
      setCashflow(await getCashflowSummary());
    } catch (err) {
      console.error("Failed to refresh cashflow:", err);
    }
  }, []);

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

  const handleSyncAccounts = async () => {
    if (syncInFlight.current) return;
    syncInFlight.current = true;
    setSyncing(true);
    setSyncError(null);
    setSyncNotice(null);
    let connected = 0;
    let synced = 0;
    let succeeded = 0;
    const problems: string[] = [];
    const providers: {
      name: string;
      status: () => Promise<{ is_connected: boolean }>;
      sync: () => Promise<{ accounts_synced: number; warnings?: string[] }>;
    }[] = [
      { name: "SimpleFIN", status: simplefinGetStatus, sync: simplefinSync },
      { name: "SnapTrade", status: snaptradeGetStatus, sync: snaptradeSync },
      { name: "Teller", status: tellerGetStatus, sync: tellerSync },
      { name: "Questrade", status: questradeGetStatus, sync: questradeSync },
    ];
    try {
      await Promise.all(providers.map(async (provider) => {
        try {
          if (!(await provider.status()).is_connected) return;
          connected += 1;
          const result = await provider.sync();
          synced += result.accounts_synced;
          succeeded += 1;
          problems.push(...(result.warnings ?? []).map((message) => `${provider.name}: ${message}`));
        } catch (err) {
          problems.push(`${provider.name}: ${String(err)}`);
        }
      }));
      if (succeeded > 0) await handleConnectorChanged();
      await refreshConnectionHealth();
      if (problems.length > 0) setSyncError(problems.join("\n"));
      if (succeeded > 0) {
        setSyncNotice(`Received updates for ${synced} account(s). Check source dates: banks can still return cached balances.`);
      } else if (connected === 0 && problems.length === 0) {
        setConnectOpen(true);
        setSyncNotice("Connect an account to enable syncing.");
      }
    } finally {
      syncInFlight.current = false;
      setSyncing(false);
    }
  };

  useEffect(() => {
    let disposed = false;
    const check = async () => {
      if (syncInFlight.current) return;
      const status = await refreshConnectionHealth();
      if (disposed || !status?.is_connected || status.app_auth_required || connectOpen || syncInFlight.current) return;
      const lastAttempt = status.last_attempt_at ? Date.parse(status.last_attempt_at) : 0;
      if (Number.isFinite(lastAttempt) && lastAttempt > Date.now() - 6 * 3600_000) return;
      syncInFlight.current = true;
      setSyncing(true);
      try {
        const result = await simplefinSync(true);
        if (!result.skipped) await handleConnectorChanged();
      } catch (err) {
        setSyncError(`SimpleFIN: ${String(err)}`);
      } finally {
        await refreshConnectionHealth();
        syncInFlight.current = false;
        setSyncing(false);
      }
    };
    void check();
    const interval = window.setInterval(() => void check(), 60_000);
    return () => { disposed = true; window.clearInterval(interval); };
  }, [connectOpen, handleConnectorChanged, refreshConnectionHealth]);

  const reviewConnections = () => {
    setConnectProvider("simplefin");
    setConnectOpen(true);
  };
  const connectionAttention = Boolean(healthError || simplefinStatus?.app_auth_required ||
    simplefinStatus?.messages.length ||
    simplefinStatus?.connections.some((bank) => bank.status !== "current"));

  const handleUpdateCurrency = useCallback(
    async (accountId: number, currency: string) => {
      await updateAccountCurrency(accountId, currency);
      // The corrected currency may need a fresh rate (e.g. JMD) to convert into net worth.
      await handleConnectorChanged();
    },
    [handleConnectorChanged],
  );

  // Net-worth-over-time series in the selected home currency.
  const chartData = history.map((point) => ({
    date: point.date,
    value: homeCurrency === "CAD" ? point.total_cad : point.total_usd,
  }));
  const connectedAccountCount = accounts.filter(
    (account) => account.connector_kind !== "manual",
  ).length;
  const todayLabel = new Intl.DateTimeFormat("en-CA", {
    weekday: "long",
    month: "long",
    day: "numeric",
  }).format(new Date());

  return (
    <div className="tn-dashboard">
      <aside className="desktop-sidebar" aria-label="TrueNorth navigation">
        <a className="desktop-sidebar__brand" href="#overview" aria-label="TrueNorth overview">
          <BrandMark className="desktop-sidebar__logo" />
        </a>

        <nav className="desktop-sidebar__nav">
          <a className="is-active" href="#overview" title="Overview">
            <DesktopIcon name="overview" />
            <span>Overview</span>
          </a>
          <a href="#activity" title="Activity and cashflow">
            <DesktopIcon name="chart" />
            <span>Activity</span>
          </a>
          <a href="#planning" title="Planning">
            <DesktopIcon name="target" />
            <span>Planning</span>
          </a>
          <a href="#accounts" title="Accounts">
            <DesktopIcon name="accounts" />
            <span>Accounts</span>
          </a>
          <button type="button" onClick={() => setConnectOpen(true)} title="Connections">
            <DesktopIcon name="connect" />
            <span>Connect</span>
          </button>
        </nav>

        <div className="desktop-sidebar__footer">
          {widgetSettings?.platform_supported && (
            <button
              type="button"
              onClick={() => setWidgetOpen(true)}
              title="Mac desktop widget"
              aria-haspopup="dialog"
            >
              <DesktopIcon name="widget" />
              <span>Widgets</span>
            </button>
          )}
          {onCheckForUpdates && (
            <button
              type="button"
              onClick={onCheckForUpdates}
              disabled={checkingUpdate}
              title="Check for a new version of TrueNorth"
            >
              <DesktopIcon name="download" />
              <span>{checkingUpdate ? "Checking" : "Updates"}</span>
            </button>
          )}
          <div className="desktop-sidebar__privacy" title="Local-first and encrypted">
            <DesktopIcon name="shield" />
          </div>
        </div>
      </aside>

      <div className="desktop-main">
        <header className="desktop-topbar">
          <div className="desktop-topbar__title">
            <span>Portfolio / Overview</span>
            <strong>TrueNorth</strong>
          </div>

          <div className={`desktop-topbar__status ${connectionAttention ? "desktop-topbar__status--attention" : ""}`}>
            <i />
            {connectionAttention ? "Connections need attention" : connectedAccountCount > 0
              ? `${connectedAccountCount} account${connectedAccountCount === 1 ? "" : "s"} linked`
              : "Local-first mode"}
          </div>

          <div className="desktop-topbar__actions">
            <button
              type="button"
              className="desktop-action"
              onClick={() => setImportOpen(true)}
              title="Import accounts and balance history"
            >
              <DesktopIcon name="upload" />
              <span>Import</span>
            </button>
            <button
              type="button"
              className="desktop-action"
              onClick={handleRefreshFx}
              disabled={refreshingFx}
              title="Refresh exchange rates"
            >
              <DesktopIcon name="refresh" />
              <span>{refreshingFx ? "Refreshing" : "Refresh FX"}</span>
            </button>
            <button
              type="button"
              className="desktop-action desktop-action--advisor"
              onClick={onToggleAdvisor}
              title="Ask the AI advisor"
            >
              <DesktopIcon name="sparkles" />
              <span>Ask advisor</span>
            </button>
            <button
              type="button"
              className="desktop-action desktop-action--primary"
              onClick={() => setConnectOpen(true)}
              title="Connect a financial institution"
            >
              <DesktopIcon name="connect" />
              <span>Connect account</span>
            </button>
          </div>
        </header>

        <main className="desktop-page">
          <section className="dashboard-intro" id="overview">
            <div>
              <span className="dashboard-intro__eyebrow">Cross-border command center</span>
              <h1>Financial overview</h1>
              <p>{todayLabel} · Every account, currency, and long-term decision in one view.</p>
            </div>
            <div className="dashboard-intro__currency">
              <span>Home currency</span>
              <strong>{homeCurrency}</strong>
            </div>
          </section>

          {loadError && <div role="alert" className="desktop-alert desktop-alert--error">{loadError}</div>}
          {(syncError || healthError) && (
            <div role="alert" className="desktop-alert desktop-alert--error">
              <span className="whitespace-pre-wrap">{syncError ?? healthError}</span>{" "}
              <button type="button" className="desktop-section-action" onClick={reviewConnections}>Review connections</button>
            </div>
          )}
          {syncNotice && <p role="status" className="mb-4 text-sm text-slate-400">{syncNotice}</p>}

        {fxError && (
          <div className="desktop-alert desktop-alert--error">
            FX refresh failed: {fxError}
          </div>
        )}

          {widgetError && (
            <div className="desktop-alert desktop-alert--error" role="alert">
              Mac widget refresh failed: {widgetError}
              <button type="button" className="desktop-action" onClick={() => void refreshWidget()}>
                Retry widget refresh
              </button>
            </div>
          )}

          <div className="desktop-layout-grid desktop-layout-grid--overview">
            <div className="desktop-span-8">
              <NetWorthCard
                netWorth={netWorth}
                delta={delta}
                homeCurrency={homeCurrency}
                onToggleCurrency={() =>
                  setHomeCurrency((currency) => (currency === "CAD" ? "USD" : "CAD"))
                }
                loading={loading}
                connectionAttention={connectionAttention}
                onReviewConnections={reviewConnections}
              />
            </div>
            <div className="desktop-span-4">
              <GoalCountdownCard goal={goal} loading={loading} />
            </div>
            <div className="desktop-span-5">
              <ProgressCard
                metrics={progress}
                loading={loading}
                onUpdate={handleUpdateProgressInputs}
              />
            </div>
            <div className="desktop-span-7">
              <CashflowCard
                summary={cashflow}
                homeCurrency={homeCurrency}
                loading={loading}
                onChanged={refreshCashflow}
              />
            </div>
            <div className="desktop-span-12">
              <DividendIncomeCard
                summary={dividends}
                homeCurrency={homeCurrency}
                loading={dividendLoading}
                error={dividendError}
                onRefresh={() => loadDividends(true)}
              />
            </div>
          </div>

          <section className="dashboard-section" id="activity">
            <div className="dashboard-section__heading">
              <div>
                <span>Movement over time</span>
                <h2>Activity & trajectory</h2>
              </div>
              <p>Separate genuine progress from currency movement and day-to-day noise.</p>
            </div>
            <NetWorthChart data={chartData} currency={homeCurrency} />
          </section>

          <section className="dashboard-section" id="planning">
            <div className="dashboard-section__heading">
              <div>
                <span>Future scenarios</span>
                <h2>Planning studio</h2>
              </div>
              <p>Turn today&apos;s complete picture into confident long-term decisions.</p>
            </div>
            <div className="desktop-layout-grid">
              <div className="desktop-span-5">
                <FirePlannerCard
                  plan={firePlan}
                  loading={loading}
                  onUpdate={handleUpdateFireInputs}
                />
              </div>
              <div className="desktop-span-7">
                <SeattleSimulatorCard
                  projection={projection}
                  loading={loading}
                  onUpdate={handleUpdateAssumptions}
                />
              </div>
            </div>
          </section>

          <section className="dashboard-section" id="accounts">
            <div className="dashboard-section__heading">
              <div>
                <span>Financial world</span>
                <h2>Accounts across borders</h2>
              </div>
              <button
                type="button"
                className="desktop-section-action"
                onClick={() => setModal({ open: true, mode: "add_account" })}
              >
                <DesktopIcon name="plus" />
                Add account
              </button>
            </div>
          <AccountList
            accounts={accounts}
            netWorthBreakdown={netWorth?.accounts ?? []}
            homeCurrency={homeCurrency}
            onAddAccount={() => setModal({ open: true, mode: "add_account" })}
            onDeleteAccount={handleDeleteAccount}
            onUpdateBalance={(account) =>
              setModal({ open: true, mode: "update_balance", account })
            }
            onEditCurrency={(account) =>
              setModal({ open: true, mode: "edit_currency", account })
            }
            simplefinHealth={simplefinStatus?.connections}
            onManageConnections={reviewConnections}
          />
          </section>

        {accounts.length === 0 && !loading && (
          <p className="desktop-privacy-note">
            <DesktopIcon name="shield" />
            Your finance database stays local and encrypted on this device.
          </p>
        )}
        </main>
      </div>

      {!connectOpen && !importOpen && !modal.open && !widgetOpen && (
        <button
          type="button"
          className="desktop-sync-fab"
          onClick={() => void handleSyncAccounts()}
          disabled={syncing}
          aria-busy={syncing}
          title="Sync connected accounts. Bank refreshes may still be pending; this is separate from Refresh FX."
        >
          <DesktopIcon name="refresh" />
          <span>{syncing ? "Syncing..." : "Sync accounts"}</span>
        </button>
      )}

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
      {modal.open && modal.mode === "edit_currency" && (
        <AccountModal
          isOpen
          mode="edit_currency"
          accountToUpdate={modal.account}
          onClose={() => setModal({ open: false })}
          onAddAccount={handleAddAccount}
          onUpdateBalance={handleUpdateBalance}
          onUpdateCurrency={handleUpdateCurrency}
        />
      )}

      <ImportModal
        isOpen={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={load}
      />

      <ConnectionsModal
        isOpen={connectOpen}
        initialProvider={connectProvider}
        onClose={() => setConnectOpen(false)}
        onChanged={handleConnectorChanged}
      />
      {widgetOpen && widgetSettings && (
        <MacWidgetModal
          settings={widgetSettings}
          onClose={() => setWidgetOpen(false)}
          onSettingsChanged={(settings) => {
            setWidgetSettings(settings);
            setWidgetError(null);
          }}
        />
      )}
    </div>
  );
}
