import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  FxRate,
  GoalProgress,
  ImportPayload,
  ImportSummary,
  NetWorth,
  NetWorthDelta,
  NetWorthHistoryPoint,
  SimpleFinStatus,
  SimpleFinSyncSummary,
  SnapTradeStatus,
  SnapTradeSyncSummary,
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

export const getFxRates = (): Promise<FxRate[]> =>
  invoke("get_fx_rates");

export const refreshFxRates = (): Promise<FxRate[]> =>
  invoke("refresh_fx_rates");

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
