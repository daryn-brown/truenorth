import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  CashflowSummary,
  ClassifiedTransaction,
  FlowType,
  FxRate,
  GoalProgress,
  ImportPayload,
  ImportSummary,
  NetWorth,
  NetWorthDelta,
  NetWorthHistoryPoint,
  SeattleAssumptions,
  SeattleProjection,
  SimpleFinStatus,
  SimpleFinSyncSummary,
  SnapTradeStatus,
  SnapTradeSyncSummary,
  TxnRule,
} from "../types/finance";

export const listAccounts = (): Promise<Account[]> =>
  invoke("list_accounts");

export const addAccount = (payload: AddAccountPayload): Promise<Account> =>
  invoke("add_account", { payload });

export const deleteAccount = (accountId: number): Promise<void> =>
  invoke("delete_account", { accountId });

export const addBalanceSnapshot = (
  payload: AddBalanceSnapshotPayload,
): Promise<BalanceSnapshotResult> =>
  invoke("add_balance_snapshot", { payload });

export const getNetWorth = (): Promise<NetWorth> =>
  invoke("get_net_worth");

export const getNetWorthHistory = (): Promise<NetWorthHistoryPoint[]> =>
  invoke("get_net_worth_history");

export const getNetWorthDelta = (): Promise<NetWorthDelta> =>
  invoke("get_net_worth_delta");

export const getGoalProgress = (): Promise<GoalProgress> =>
  invoke("get_goal_progress");

/** Update the headline net-worth milestone (USD); returns recomputed progress. */
export const setGoalTarget = (targetUsd: number): Promise<GoalProgress> =>
  invoke("set_goal_target", { targetUsd });

/** Forward net-worth projection: status quo vs. dropping Job 2 in the Seattle move. */
export const getSeattleProjection = (): Promise<SeattleProjection> =>
  invoke("get_seattle_projection");
/** Persist edited Seattle assumptions; returns the recomputed projection. */
export const setSeattleAssumptions = (
  assumptions: SeattleAssumptions,
): Promise<SeattleProjection> =>
  invoke("set_seattle_assumptions", { assumptions });

// --- Cashflow + fixed/variable tagging (Step 5) ----------------------------

/** Income vs. fixed vs. variable spending + savings rate over the trailing window (default 30d). */
export const getCashflowSummary = (
  windowDays?: number,
): Promise<CashflowSummary> =>
  invoke("get_cashflow_summary", { windowDays });

/** Recent transactions with their resolved flow type, for review and retagging. */
export const listRecentTransactions = (
  limit?: number,
): Promise<ClassifiedTransaction[]> =>
  invoke("list_recent_transactions", { limit });

/** Set (or clear, with null) a transaction's manual fixed/variable/income/transfer override. */
export const setTransactionFlow = (
  transactionId: number,
  flowType: FlowType | null,
): Promise<void> =>
  invoke("set_transaction_flow", { transactionId, flowType });

export const listTxnRules = (): Promise<TxnRule[]> =>
  invoke("list_txn_rules");

export const addTxnRule = (
  pattern: string,
  flowType: FlowType,
): Promise<TxnRule> =>
  invoke("add_txn_rule", { pattern, flowType });

export const deleteTxnRule = (ruleId: number): Promise<void> =>
  invoke("delete_txn_rule", { ruleId });

export const getFxRates = (): Promise<FxRate[]> =>
  invoke("get_fx_rates");

export const refreshFxRates = (): Promise<FxRate[]> =>
  invoke("refresh_fx_rates");

/** Refresh FX only when the stored rates aren't from today — a best-effort daily auto-refresh. */
export const refreshFxRatesIfStale = (): Promise<FxRate[]> =>
  invoke("refresh_fx_rates_if_stale");

export const importData = (payload: ImportPayload): Promise<ImportSummary> =>
  invoke("import_data", { payload });

// --- SnapTrade (Phase 2) ---------------------------------------------------

export const snaptradeGetStatus = (): Promise<SnapTradeStatus> =>
  invoke("snaptrade_get_status");

export const snaptradeSaveCredentials = (
  clientId: string,
  consumerKey: string,
): Promise<SnapTradeStatus> =>
  invoke("snaptrade_save_credentials", { clientId, consumerKey });

/** List SnapTrade user IDs registered under the saved key (one, for personal keys). */
export const snaptradeListUsers = (): Promise<string[]> =>
  invoke("snaptrade_list_users");

/** Link a personal SnapTrade user with credentials copied from the dashboard. */
export const snaptradeLinkUser = (
  userId: string,
  userSecret: string,
): Promise<SnapTradeStatus> =>
  invoke("snaptrade_link_user", { userId, userSecret });

export const snaptradeGetLoginLink = (): Promise<string> =>
  invoke("snaptrade_get_login_link");

export const snaptradeSync = (): Promise<SnapTradeSyncSummary> =>
  invoke("snaptrade_sync");

export const snaptradeDisconnect = (): Promise<SnapTradeStatus> =>
  invoke("snaptrade_disconnect");

// --- SimpleFIN (Phase 3) ---------------------------------------------------

export const simplefinGetStatus = (): Promise<SimpleFinStatus> =>
  invoke("simplefin_get_status");

/** Claim a SimpleFIN setup token and store the resulting access URL. */
export const simplefinConnect = (setupToken: string): Promise<SimpleFinStatus> =>
  invoke("simplefin_connect", { setupToken });

export const simplefinSync = (): Promise<SimpleFinSyncSummary> =>
  invoke("simplefin_sync");

export const simplefinDisconnect = (): Promise<SimpleFinStatus> =>
  invoke("simplefin_disconnect");

interface BalanceSnapshotResult {
  id: number;
  account_id: number;
  snapshot_date: string;
  balance: number;
  currency: string;
}
