import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  FxRate,
  NetWorth,
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

export const getFxRates = (): Promise<FxRate[]> =>
  invoke("get_fx_rates");

export const refreshFxRates = (): Promise<FxRate[]> =>
  invoke("refresh_fx_rates");

interface BalanceSnapshotResult {
  id: number;
  account_id: number;
  snapshot_date: string;
  balance: number;
  currency: string;
}
