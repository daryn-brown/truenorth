import { useCallback, useEffect, useState } from "react";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import type {
  QuestradeStatus,
  QuestradeSyncSummary,
  SimpleFinStatus,
  SimpleFinSyncSummary,
  SnapTradeStatus,
  SnapTradeSyncSummary,
} from "../types/finance";
import {
  questradeConnect,
  questradeDisconnect,
  questradeGetStatus,
  questradeSync,
  simplefinConnect,
  simplefinDisconnect,
  simplefinGetStatus,
  simplefinSync,
  snaptradeDisconnect,
  snaptradeGetLoginLink,
  snaptradeGetStatus,
  snaptradeLinkUser,
  snaptradeListUsers,
  snaptradeSaveCredentials,
  snaptradeSync,
} from "../hooks/useFinanceApi";

interface Props {
  isOpen: boolean;
  onClose: () => void;
  /** Called after a successful sync or disconnect so the dashboard can reload. */
  onChanged: () => void;
}

type Provider = "snaptrade" | "simplefin" | "direct";

const SNAPTRADE_DASHBOARD = "https://dashboard.snaptrade.com";
const SIMPLEFIN_BRIDGE = "https://bridge.simplefin.org";
const QUESTRADE_API_CENTRE = "https://login.questrade.com/APIAccess/UserApps.aspx";

const inputClass =
  "w-full rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm text-slate-200 placeholder-slate-600 focus:outline-none focus:ring-2 focus:ring-indigo-500";

export default function ConnectionsModal({ isOpen, onClose, onChanged }: Props) {
  const [provider, setProvider] = useState<Provider>("snaptrade");

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="w-full max-w-lg rounded-2xl border border-slate-700 bg-slate-900 p-6 shadow-2xl">
        <h2 className="mb-1 text-lg font-semibold text-white">Connect accounts</h2>
        <p className="mb-4 text-xs text-slate-400">
          Sync real balances automatically instead of entering them by hand. TrueNorth requests{" "}
          <span className="font-semibold text-slate-300">read-only</span> access only — it can never
          move money. Secrets are stored locally on this device, in your app data folder.
        </p>

        <div className="mb-5 grid grid-cols-3 gap-1 rounded-lg border border-slate-700 bg-slate-800/60 p-1">
          <TabButton
            active={provider === "snaptrade"}
            onClick={() => setProvider("snaptrade")}
            label="Brokerages"
            hint="via SnapTrade"
          />
          <TabButton
            active={provider === "simplefin"}
            onClick={() => setProvider("simplefin")}
            label="Banks"
            hint="via SimpleFIN"
          />
          <TabButton
            active={provider === "direct"}
            onClick={() => setProvider("direct")}
            label="Direct"
            hint="Institution APIs"
          />
        </div>

        {provider === "snaptrade" ? (
          <SnapTradePanel onChanged={onChanged} />
        ) : provider === "simplefin" ? (
          <SimpleFinPanel onChanged={onChanged} />
        ) : (
          <DirectConnectionsPanel onChanged={onChanged} />
        )}

        <div className="flex justify-end pt-5">
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

// ---------------------------------------------------------------------------
// SnapTrade — brokerages
// ---------------------------------------------------------------------------

function SnapTradePanel({ onChanged }: { onChanged: () => void }) {
  const [status, setStatus] = useState<SnapTradeStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [consumerKey, setConsumerKey] = useState("");
  const [userId, setUserId] = useState("");
  const [userSecret, setUserSecret] = useState("");
  const [relinking, setRelinking] = useState(false);
  const [busy, setBusy] = useState<
    null | "save" | "lookup" | "link" | "connect" | "sync" | "disconnect"
  >(null);
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
    void refreshStatus();
  }, [refreshStatus]);

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

  const handleLookupUser = async () => {
    setBusy("lookup");
    setError(null);
    setInfo(null);
    try {
      const users = await snaptradeListUsers();
      if (users.length > 0) {
        setUserId(users[0]);
        setInfo(
          users.length === 1
            ? "Found your SnapTrade User ID. Now paste your User Secret."
            : `Found ${users.length} users — filled in the first. Edit if needed.`,
        );
      } else {
        setError("No SnapTrade user is registered to this key yet.");
      }
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const handleLinkUser = async () => {
    setBusy("link");
    setError(null);
    setInfo(null);
    try {
      const next = await snaptradeLinkUser(userId, userSecret);
      setStatus(next);
      setUserSecret("");
      setRelinking(false);
      setInfo("SnapTrade user linked. You can sync now.");
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
  const isPersonal = status?.is_personal ?? false;
  const showLinkForm = isPersonal && (!isConnected || relinking);

  return (
    <>
      <p className="mb-4 text-xs text-slate-400">
        Connect Robinhood, Questrade, Wealthsimple and more via{" "}
        <button
          type="button"
          onClick={() => void openUrl(SNAPTRADE_DASHBOARD)}
          className="text-indigo-400 underline hover:text-indigo-300"
        >
          SnapTrade
        </button>
        .
      </p>

      {/* Step 1 — API credentials */}
      <Section step={1} title="SnapTrade API key" done={hasCredentials}>
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
              Create a free developer account, then copy your Client ID and Consumer Key from the{" "}
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

      {/* Step 2 — SnapTrade user */}
      <Section step={2} title="SnapTrade user" done={isConnected} disabled={!hasCredentials}>
        {showLinkForm ? (
          <div className="space-y-3">
            <p className="text-xs text-slate-500">
              Personal SnapTrade keys (
              <span className="font-mono text-slate-400">PERS-…</span>) come with a user that's
              created for you. Copy your <span className="text-slate-300">User ID</span> and{" "}
              <span className="text-slate-300">User Secret</span> from the{" "}
              <button
                type="button"
                onClick={() => void openUrl(SNAPTRADE_DASHBOARD)}
                className="text-indigo-400 underline hover:text-indigo-300"
              >
                SnapTrade dashboard
              </button>
              .
            </p>
            <div className="flex gap-2">
              <input
                value={userId}
                onChange={(e) => setUserId(e.target.value)}
                placeholder="User ID"
                spellCheck={false}
                className={inputClass}
              />
              <button
                type="button"
                onClick={handleLookupUser}
                disabled={busy !== null}
                title="Look up the User ID registered to your key"
                className="shrink-0 rounded-lg border border-slate-600 px-3 text-xs font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50 transition-colors"
              >
                {busy === "lookup" ? "…" : "Find mine"}
              </button>
            </div>
            <input
              value={userSecret}
              onChange={(e) => setUserSecret(e.target.value)}
              placeholder="User Secret"
              type="password"
              spellCheck={false}
              className={inputClass}
            />
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleLinkUser}
                disabled={busy !== null || !userId.trim() || !userSecret.trim()}
                className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {busy === "link" ? "Linking…" : "Link user"}
              </button>
              {relinking && (
                <button
                  type="button"
                  onClick={() => setRelinking(false)}
                  className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
                >
                  Cancel
                </button>
              )}
            </div>
          </div>
        ) : isConnected ? (
          <div className="flex items-center justify-between gap-3 text-sm text-slate-300">
            <span className="truncate">Linked to SnapTrade</span>
            {isPersonal && (
              <button
                type="button"
                onClick={() => setRelinking(true)}
                className="shrink-0 text-xs text-slate-400 underline hover:text-slate-200"
              >
                Update secret
              </button>
            )}
          </div>
        ) : (
          <p className="text-xs text-slate-500">
            A SnapTrade user is created automatically when you connect a brokerage below.
          </p>
        )}
      </Section>

      {/* Step 3 — Authorize a brokerage */}
      <Section
        step={3}
        title="Authorize your brokerage"
        done={(status?.account_count ?? 0) > 0}
        disabled={isPersonal ? !isConnected : !hasCredentials}
      >
        <p className="mb-3 text-xs text-slate-500">
          Opens SnapTrade's secure connection portal in your browser to link an institution.
          {isPersonal
            ? " Already linked a brokerage in the SnapTrade dashboard? You can skip straight to Sync."
            : ""}
        </p>
        <button
          type="button"
          onClick={handleConnect}
          disabled={busy !== null || (isPersonal ? !isConnected : !hasCredentials)}
          className="w-full rounded-lg border border-slate-600 py-2 text-sm font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50 transition-colors"
        >
          {busy === "connect"
            ? "Opening…"
            : isConnected
              ? "Connect another brokerage"
              : "Connect a brokerage"}
        </button>
      </Section>

      {/* Step 4 — Sync */}
      <Section step={4} title="Sync balances" disabled={!isConnected} last>
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

      <Feedback info={info} error={error} />

      {isConnected && (
        <div className="pt-4">
          <button
            type="button"
            onClick={handleDisconnect}
            disabled={busy !== null}
            className="text-xs font-medium text-red-400 hover:text-red-300 disabled:opacity-50"
          >
            {busy === "disconnect" ? "Disconnecting…" : "Disconnect brokerage"}
          </button>
        </div>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// SimpleFIN — banks
// ---------------------------------------------------------------------------

function SimpleFinPanel({ onChanged }: { onChanged: () => void }) {
  const [status, setStatus] = useState<SimpleFinStatus | null>(null);
  const [setupToken, setSetupToken] = useState("");
  const [reclaiming, setReclaiming] = useState(false);
  const [busy, setBusy] = useState<null | "connect" | "sync" | "disconnect">(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [summary, setSummary] = useState<SimpleFinSyncSummary | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await simplefinGetStatus());
    } catch (err) {
      setError(messageOf(err));
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const handleConnect = async () => {
    setBusy("connect");
    setError(null);
    setInfo(null);
    try {
      const next = await simplefinConnect(setupToken);
      setStatus(next);
      setSetupToken("");
      setReclaiming(false);
      setInfo("SimpleFIN connected. Click “Sync now” to pull balances.");
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
      const result = await simplefinSync();
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
        "Disconnect SimpleFIN? Connected accounts will be hidden and synced balances stop updating. Your history is kept.",
      )
    ) {
      return;
    }
    setBusy("disconnect");
    setError(null);
    setInfo(null);
    try {
      const next = await simplefinDisconnect();
      setStatus(next);
      setSummary(null);
      onChanged();
      setInfo("SimpleFIN disconnected.");
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const isConnected = status?.is_connected ?? false;
  const showTokenForm = !isConnected || reclaiming;

  return (
    <>
      <p className="mb-4 text-xs text-slate-400">
        Connect banks and other institutions through{" "}
        <button
          type="button"
          onClick={() => void openUrl(SIMPLEFIN_BRIDGE)}
          className="text-indigo-400 underline hover:text-indigo-300"
        >
          SimpleFIN
        </button>
        . Create a setup token in your bridge, then paste it below.
      </p>

      {/* Step 1 — Setup token */}
      <Section step={1} title="SimpleFIN setup token" done={isConnected}>
        {showTokenForm ? (
          <div className="space-y-3">
            <p className="text-xs text-slate-500">
              In your{" "}
              <button
                type="button"
                onClick={() => void openUrl(SIMPLEFIN_BRIDGE)}
                className="text-indigo-400 underline hover:text-indigo-300"
              >
                SimpleFIN bridge
              </button>
              , connect your bank and click <span className="text-slate-300">Connect</span> to
              generate a one-time setup token, then paste it here.
            </p>
            <textarea
              value={setupToken}
              onChange={(e) => setSetupToken(e.target.value)}
              placeholder="Paste your setup token (a long string of letters and numbers)"
              spellCheck={false}
              rows={3}
              className={`${inputClass} resize-none font-mono text-xs`}
            />
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleConnect}
                disabled={busy !== null || !setupToken.trim()}
                className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {busy === "connect" ? "Connecting…" : "Connect"}
              </button>
              {reclaiming && (
                <button
                  type="button"
                  onClick={() => {
                    setReclaiming(false);
                    setSetupToken("");
                  }}
                  className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
                >
                  Cancel
                </button>
              )}
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-3 text-sm text-slate-300">
            <span className="truncate">Connected to SimpleFIN</span>
            <button
              type="button"
              onClick={() => setReclaiming(true)}
              className="shrink-0 text-xs text-slate-400 underline hover:text-slate-200"
            >
              Use a new token
            </button>
          </div>
        )}
      </Section>

      {/* Step 2 — Sync */}
      <Section step={2} title="Sync balances" disabled={!isConnected} last>
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
          <div className="mt-3 space-y-2">
            <p className="rounded-lg bg-emerald-900/20 border border-emerald-700/40 px-3 py-2 text-xs text-emerald-300">
              Synced {summary.accounts_synced} account
              {summary.accounts_synced === 1 ? "" : "s"}, {summary.holdings_synced} holding
              {summary.holdings_synced === 1 ? "" : "s"}, and {summary.transactions_synced}{" "}
              transaction{summary.transactions_synced === 1 ? "" : "s"}. Net worth is up to date.
            </p>
            {summary.warnings.length > 0 && (
              <ul className="rounded-lg bg-amber-900/20 border border-amber-700/40 px-3 py-2 text-xs text-amber-300 space-y-1">
                {summary.warnings.map((w, i) => (
                  <li key={i}>• {w}</li>
                ))}
              </ul>
            )}
          </div>
        )}
      </Section>

      <Feedback info={info} error={error} />

      {isConnected && (
        <div className="pt-4">
          <button
            type="button"
            onClick={handleDisconnect}
            disabled={busy !== null}
            className="text-xs font-medium text-red-400 hover:text-red-300 disabled:opacity-50"
          >
            {busy === "disconnect" ? "Disconnecting…" : "Disconnect SimpleFIN"}
          </button>
        </div>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Direct — individual institution APIs
// ---------------------------------------------------------------------------

/**
 * A home for institutions you connect through their own API instead of an aggregator. Each one is a
 * self-contained card; add another institution by dropping a new card alongside `QuestradeConnection`.
 */
function DirectConnectionsPanel({ onChanged }: { onChanged: () => void }) {
  return (
    <>
      <p className="mb-4 text-xs text-slate-400">
        Connect an institution’s own API directly — no aggregator in between. Use this when a broker
        or bank offers a personal API, or when an aggregator can’t see your full balance (for
        example, SimpleFIN reports only the cash in a Questrade account, not the stock equity). Each
        connection is <span className="font-semibold text-slate-300">read-only</span> and its secret
        is stored locally on this device.
      </p>

      <QuestradeConnection onChanged={onChanged} />
    </>
  );
}

function QuestradeConnection({ onChanged }: { onChanged: () => void }) {
  const [status, setStatus] = useState<QuestradeStatus | null>(null);
  const [refreshToken, setRefreshToken] = useState("");
  const [reconnecting, setReconnecting] = useState(false);
  const [busy, setBusy] = useState<null | "connect" | "sync" | "disconnect">(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [summary, setSummary] = useState<QuestradeSyncSummary | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await questradeGetStatus());
    } catch (err) {
      setError(messageOf(err));
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const handleConnect = async () => {
    setBusy("connect");
    setError(null);
    setInfo(null);
    try {
      const next = await questradeConnect(refreshToken);
      setStatus(next);
      setRefreshToken("");
      setReconnecting(false);
      setInfo("Questrade connected. Click “Sync now” to pull balances and holdings.");
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
      const result = await questradeSync();
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
        "Disconnect Questrade? Connected accounts will be hidden and synced balances stop updating. Your history is kept.",
      )
    ) {
      return;
    }
    setBusy("disconnect");
    setError(null);
    setInfo(null);
    try {
      const next = await questradeDisconnect();
      setStatus(next);
      setSummary(null);
      onChanged();
      setInfo("Questrade disconnected.");
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(null);
    }
  };

  const isConnected = status?.is_connected ?? false;
  const showTokenForm = !isConnected || reconnecting;

  return (
    <div className="rounded-xl border border-slate-700 bg-slate-800/40 p-4">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-white">Questrade</h3>
          <p className="text-[11px] text-slate-500">Direct API · pulls cash and stock equity</p>
        </div>
        <span
          className={`shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium ${
            isConnected
              ? "bg-emerald-900/40 text-emerald-300"
              : "border border-slate-600 text-slate-400"
          }`}
        >
          {isConnected ? "Connected" : "Not connected"}
        </span>
      </div>

      {/* Step 1 — Refresh token */}
      <Section step={1} title="Questrade refresh token" done={isConnected}>
        {showTokenForm ? (
          <div className="space-y-3">
            <p className="text-xs text-slate-500">
              In your{" "}
              <button
                type="button"
                onClick={() => void openUrl(QUESTRADE_API_CENTRE)}
                className="text-indigo-400 underline hover:text-indigo-300"
              >
                Questrade API Centre
              </button>
              , register a personal app, run a manual authorization, and paste the refresh token
              here. Questrade rotates it on every sync and TrueNorth stores the latest one; if it
              goes unused for ~7 days, generate a new one.
            </p>
            <textarea
              value={refreshToken}
              onChange={(e) => setRefreshToken(e.target.value)}
              placeholder="Paste your refresh token (a short string of letters and numbers)"
              spellCheck={false}
              rows={2}
              className={`${inputClass} resize-none font-mono text-xs`}
            />
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleConnect}
                disabled={busy !== null || !refreshToken.trim()}
                className="flex-1 rounded-lg bg-indigo-600 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {busy === "connect" ? "Connecting…" : "Connect"}
              </button>
              {reconnecting && (
                <button
                  type="button"
                  onClick={() => {
                    setReconnecting(false);
                    setRefreshToken("");
                  }}
                  className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-700 transition-colors"
                >
                  Cancel
                </button>
              )}
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-3 text-sm text-slate-300">
            <span className="truncate">Connected to Questrade</span>
            <button
              type="button"
              onClick={() => setReconnecting(true)}
              className="shrink-0 text-xs text-slate-400 underline hover:text-slate-200"
            >
              Use a new token
            </button>
          </div>
        )}
      </Section>

      {/* Step 2 — Sync */}
      <Section step={2} title="Sync balances" disabled={!isConnected} last>
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

        {isConnected && (
          <p className="mt-2 text-[11px] text-slate-600">
            Full equity (cash + market value) flows into net worth. A duplicate cash-only Questrade
            account from another connection is hidden automatically; if one remains, hide it from the
            Accounts list.
          </p>
        )}

        {summary && (
          <div className="mt-3 space-y-2">
            <p className="rounded-lg bg-emerald-900/20 border border-emerald-700/40 px-3 py-2 text-xs text-emerald-300">
              Synced {summary.accounts_synced} account
              {summary.accounts_synced === 1 ? "" : "s"} and {summary.holdings_synced} holding
              {summary.holdings_synced === 1 ? "" : "s"}. Net worth is up to date.
            </p>
            {summary.duplicates_hidden > 0 && (
              <p className="rounded-lg bg-slate-700/30 border border-slate-600/50 px-3 py-2 text-xs text-slate-300">
                Hid {summary.duplicates_hidden} duplicate Questrade account
                {summary.duplicates_hidden === 1 ? "" : "s"} that another connection (e.g. SimpleFIN)
                was importing with cash only, so net worth isn’t double-counted.
              </p>
            )}
          </div>
        )}
      </Section>

      <Feedback info={info} error={error} />

      {isConnected && (
        <div className="pt-4">
          <button
            type="button"
            onClick={handleDisconnect}
            disabled={busy !== null}
            className="text-xs font-medium text-red-400 hover:text-red-300 disabled:opacity-50"
          >
            {busy === "disconnect" ? "Disconnecting…" : "Disconnect Questrade"}
          </button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shared UI
// ---------------------------------------------------------------------------

function TabButton({
  active,
  onClick,
  label,
  hint,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  hint: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-md px-3 py-2 text-left transition-colors ${
        active ? "bg-indigo-600 text-white" : "text-slate-300 hover:bg-slate-700/60"
      }`}
    >
      <span className="block text-sm font-semibold">{label}</span>
      <span className={`block text-[11px] ${active ? "text-indigo-100" : "text-slate-500"}`}>
        {hint}
      </span>
    </button>
  );
}

function Feedback({ info, error }: { info: string | null; error: string | null }) {
  return (
    <>
      {info && (
        <p className="mt-4 rounded-lg bg-slate-800 border border-slate-700 px-3 py-2 text-xs text-slate-300">
          {info}
        </p>
      )}
      {error && (
        <p className="mt-4 rounded-lg bg-red-900/20 px-3 py-2 text-sm text-red-400">{error}</p>
      )}
    </>
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
            done ? "bg-emerald-600 text-white" : "border border-slate-600 text-slate-400"
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
