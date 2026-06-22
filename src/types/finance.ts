/** All money-carrying values include their currency. */
export type Currency = "USD" | "CAD";

export type Jurisdiction = "US" | "CA";

export type ConnectorKind = "manual" | "snaptrade" | "simplefin" | "questrade";

/** Matches the `account_types` reference table. */
export type AccountTypeId =
  | "chequing"
  | "savings"
  | "brokerage"
  | "tfsa"
  | "rrsp"
  | "fhsa"
  | "401k"
  | "ira"
  | "roth_ira"
  | "credit"
  | "crypto"
  | "other";

export interface AccountType {
  id: AccountTypeId;
  label: string;
  category: "banking" | "investment" | "retirement" | "credit" | "crypto";
}

export interface Account {
  id: number;
  name: string;
  institution: string;
  account_type: AccountTypeId;
  currency: Currency;
  jurisdiction: Jurisdiction;
  connector_kind: ConnectorKind;
  connector_ref: string | null;
  is_active: boolean;
  notes: string | null;
  created_at: string;
  updated_at: string;
  /** Latest balance — joined when fetching the account list. */
  latest_balance?: number | null;
  latest_balance_date?: string | null;
}

export interface BalanceSnapshot {
  id: number;
  account_id: number;
  snapshot_date: string;
  balance: number;
  currency: Currency;
  source: string;
  created_at: string;
}

export interface FxRate {
  id: number;
  from_currency: Currency;
  to_currency: Currency;
  rate: number;
  rate_date: string;
  source: string;
  created_at: string;
}

export interface Holding {
  id: number;
  account_id: number;
  symbol: string;
  quantity: number;
  average_cost: number | null;
  currency: Currency;
  last_price: number | null;
  last_price_at: string | null;
  updated_at: string;
}

export interface Goal {
  id: number;
  name: string;
  target_amount: number;
  currency: Currency;
  target_date: string | null;
  linked_account_ids: number[];
  notes: string | null;
  created_at: string;
}

export interface NetWorth {
  /** Total net worth in USD. */
  total_usd: number;
  /** Total net worth in CAD. */
  total_cad: number;
  /** Per-account breakdown. */
  accounts: AccountNetWorth[];
  /** The USD→CAD rate used (or null if unavailable). */
  usd_cad_rate: number | null;
  /** The CAD→USD rate used (or null if unavailable). */
  cad_usd_rate: number | null;
  /** ISO date string of the rates used. */
  rate_date: string | null;
}

/** One point in the net-worth-over-time series. */
export interface NetWorthHistoryPoint {
  /** ISO date YYYY-MM-DD. */
  date: string;
  total_usd: number;
  total_cad: number;
}

/** A money figure in both reporting currencies, mirrored from the Rust `MoneyPair`. */
export interface MoneyPair {
  usd: number;
  cad: number;
}

/**
 * Change in net worth since the previous snapshot date, split into spendable cash vs.
 * investments. Powers the "Anxiety Buffer" reassurance line. Mirrors the Rust `NetWorthDelta`.
 */
export interface NetWorthDelta {
  current_date: string | null;
  previous_date: string | null;
  total: MoneyPair;
  liquid: MoneyPair;
  invested: MoneyPair;
  total_delta: MoneyPair;
  liquid_delta: MoneyPair;
  invested_delta: MoneyPair;
  has_previous: boolean;
}

export interface AccountNetWorth {
  account_id: number;
  account_name: string;
  institution: string;
  account_type: AccountTypeId;
  jurisdiction: Jurisdiction;
  balance: number;
  currency: Currency;
  balance_usd: number;
  balance_cad: number;
  snapshot_date: string | null;
}

export interface AddAccountPayload {
  name: string;
  institution: string;
  account_type: AccountTypeId;
  currency: Currency;
  jurisdiction: Jurisdiction;
  notes: string | null;
}

export interface AddBalanceSnapshotPayload {
  account_id: number;
  balance: number;
  snapshot_date: string;
}

/** One historical balance for an imported account. */
export interface ImportSnapshotInput {
  snapshot_date: string;
  balance: number;
}

/** An account (with optional history) in an import payload. */
export interface ImportAccountInput {
  name: string;
  institution: string;
  account_type: AccountTypeId;
  currency: Currency;
  jurisdiction: Jurisdiction;
  notes?: string | null;
  snapshots: ImportSnapshotInput[];
}

export interface ImportPayload {
  accounts: ImportAccountInput[];
}

export interface ImportSummary {
  accounts_created: number;
  accounts_matched: number;
  snapshots_imported: number;
  errors: string[];
}

/** SnapTrade connection state, mirrored from the Rust `SnapTradeStatus`. */
export interface SnapTradeStatus {
  /** API key pair saved (clientId + consumerKey). */
  has_credentials: boolean;
  /** A brokerage is connected (SnapTrade user exists). */
  is_connected: boolean;
  /**
   * The clientId is a personal SnapTrade key (`PERS-…`): its user is auto-provisioned at
   * signup, so the user links a userId + userSecret instead of the app registering one.
   */
  is_personal: boolean;
  /** Public clientId, for display. Never the secret consumerKey. */
  client_id: string | null;
  last_synced_at: string | null;
  account_count: number;
}

/** Result of a SnapTrade sync, mirrored from the Rust `SnapTradeSyncSummary`. */
export interface SnapTradeSyncSummary {
  accounts_synced: number;
  holdings_synced: number;
  synced_at: string;
}

/** SimpleFIN connection state, mirrored from the Rust `SimpleFinStatus`. */
export interface SimpleFinStatus {
  /** An access URL is stored (a setup token has been claimed). */
  is_connected: boolean;
  last_synced_at: string | null;
  /** Number of active accounts connected via SimpleFIN. */
  account_count: number;
}

/** Result of a SimpleFIN sync, mirrored from the Rust `SimpleFinSyncSummary`. */
export interface SimpleFinSyncSummary {
  accounts_synced: number;
  holdings_synced: number;
  synced_at: string;
  /** Non-fatal messages SimpleFIN returned (e.g. an institution needs re-auth). */
  warnings: string[];
}
