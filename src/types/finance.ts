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
  /** ISO-4217 code. Usually USD/CAD, but a connected account can be any currency (e.g. JMD). */
  currency: string;
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
  currency: string;
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

/**
 * Progress toward the headline net-worth milestone (USD) with a projected hit-date.
 * Mirrors the Rust `GoalProgress`.
 */
export interface GoalProgress {
  target_usd: number;
  current_usd: number;
  gap_usd: number;
  /** Fraction complete, 0..1. */
  progress: number;
  already_met: boolean;
  /** Net-worth change per day over the trailing ~30 days, or null with too little history. */
  daily_rate_usd: number | null;
  /** That pace per 30 days, for display. */
  monthly_rate_usd: number | null;
  /** Actual span (days) the pace was measured over. */
  window_days: number | null;
  /** Projected date net worth reaches the target (YYYY-MM-DD), or null when unprojectable. */
  projected_date: string | null;
  days_to_goal: number | null;
}

/** Editable levers behind the Seattle projection. Mirrors the Rust `SeattleAssumptions`. */
export interface SeattleAssumptions {
  current_net_monthly_usd: number;
  current_expenses_monthly_usd: number;
  seattle_net_monthly_usd: number;
  seattle_expenses_monthly_usd: number;
  transition_months: number;
  horizon_months: number;
  annual_return_pct: number;
}

/** One month on the Seattle projection. Month 0 is "today" (both scenarios start equal). */
export interface ProjectionPoint {
  month: number;
  date: string;
  current_usd: number;
  seattle_usd: number;
}

/**
 * Forward net-worth projection under the status-quo vs. Seattle (drop Job 2) scenarios.
 * Mirrors the Rust `SeattleProjection`.
 */
export interface SeattleProjection {
  start_usd: number;
  start_date: string;
  transition_date: string;
  current_monthly_contribution_usd: number;
  seattle_monthly_contribution_usd: number;
  current_end_usd: number;
  seattle_end_usd: number;
  /** seattle_end − current_end (negative = the cost of dropping Job 2 over the horizon). */
  end_gap_usd: number;
  points: ProjectionPoint[];
  assumptions: SeattleAssumptions;
}

export interface AccountNetWorth {
  account_id: number;
  account_name: string;
  institution: string;
  account_type: AccountTypeId;
  jurisdiction: Jurisdiction;
  balance: number;
  currency: string;
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
  transactions_synced: number;
  synced_at: string;
  /** Non-fatal messages SimpleFIN returned (e.g. an institution needs re-auth). */
  warnings: string[];
}

/** How a transaction is counted toward cashflow. Mirrors the Rust `FlowType`. */
export type FlowType = "income" | "fixed" | "variable" | "transfer";

/** A classification rule (case-insensitive substring of a description). Mirrors `TxnRule`. */
export interface TxnRule {
  id: number;
  pattern: string;
  flow_type: FlowType;
}

/** A transaction with its resolved flow type. Mirrors the Rust `ClassifiedTransaction`. */
export interface ClassifiedTransaction {
  id: number;
  account_id: number;
  account_name: string;
  txn_date: string;
  description: string;
  amount: number;
  currency: Currency;
  flow_type: FlowType;
  /** True when the flow type comes from a manual override rather than a rule/sign default. */
  is_override: boolean;
}

/**
 * Rolling-window cashflow totals separating fixed commitments from variable "lifestyle"
 * spending, with transfers excluded. Mirrors the Rust `CashflowSummary`.
 */
export interface CashflowSummary {
  window_days: number;
  since: string;
  income: MoneyPair;
  fixed: MoneyPair;
  variable: MoneyPair;
  net_savings: MoneyPair;
  /** net_savings / income (USD basis), 0 when there was no income. */
  savings_rate: number;
  transfer_count: number;
  txn_count: number;
  /** True when some transaction's currency had no FX rate (counted as 0). */
  currency_warning: boolean;
}

/** Questrade direct-connection state, mirrored from the Rust `QuestradeStatus`. */
export interface QuestradeStatus {
  /** A refresh token is stored (the Questrade API app is connected). */
  is_connected: boolean;
  last_synced_at: string | null;
  /** Number of active accounts connected directly via Questrade. */
  account_count: number;
}

/** Result of a Questrade sync, mirrored from the Rust `QuestradeSyncSummary`. */
export interface QuestradeSyncSummary {
  accounts_synced: number;
  holdings_synced: number;
  /** Redundant aggregator duplicates (same account via SimpleFIN/SnapTrade) that were hidden. */
  duplicates_hidden: number;
  synced_at: string;
}
