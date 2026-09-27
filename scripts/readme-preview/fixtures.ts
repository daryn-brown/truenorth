import type {
  Account, CashflowSummary, DividendSummary, FirePlan, GoalProgress,
  MacWidgetSettings, MoneyPair, NetWorth, NetWorthDelta, NetWorthHistoryPoint,
  ProgressMetrics, SeattleProjection,
} from "../../src/types/finance";
import type { AiSettings } from "../../src/types/ai";

const date = "2026-09-24";
const timestamp = `${date}T12:00:00Z`;
const rate = 1.35;
const money = (usd: number): MoneyPair => ({ usd, cad: usd * rate });
const total = 185_400;

const accountRows = [
  { name: "US brokerage", institution: "Harbor Investing", currency: "USD", balance: 78_000, account_type: "brokerage", jurisdiction: "US", connector_kind: "snaptrade" },
  { name: "Canadian TFSA", institution: "Maple Investing", currency: "CAD", balance: 54_000, account_type: "tfsa", jurisdiction: "CA", connector_kind: "snaptrade" },
  { name: "High-yield savings", institution: "Harbor Bank", currency: "USD", balance: 28_000, account_type: "savings", jurisdiction: "US", connector_kind: "simplefin" },
  { name: "Retirement savings", institution: "Maple Investing", currency: "CAD", balance: 43_200, account_type: "rrsp", jurisdiction: "CA", connector_kind: "questrade" },
  { name: "Everyday chequing", institution: "Maple Bank", currency: "CAD", balance: 9_990, account_type: "chequing", jurisdiction: "CA", connector_kind: "simplefin" },
] as const;

const accounts: Account[] = accountRows.map((row, index) => ({
  id: index + 1,
  name: row.name,
  institution: row.institution,
  account_type: row.account_type,
  currency: row.currency,
  jurisdiction: row.jurisdiction,
  connector_kind: row.connector_kind,
  connector_ref: null,
  is_active: true,
  notes: null,
  created_at: timestamp,
  updated_at: timestamp,
  latest_balance: row.balance,
  latest_balance_date: date,
}));

const netWorth: NetWorth = {
  total_usd: total,
  total_cad: total * rate,
  allocation: {
    investments: money(150_000),
    savings: money(35_400),
    liabilities: money(0),
    unclassified: money(0),
  },
  usd_cad_rate: rate,
  cad_usd_rate: 1 / rate,
  rate_date: date,
  accounts: accountRows.map((row, index) => {
    const usd = row.currency === "USD" ? row.balance : row.balance / rate;
    return {
      account_id: index + 1, account_name: row.name, institution: row.institution,
      account_type: row.account_type, jurisdiction: row.jurisdiction,
      currency: row.currency, balance: row.balance,
      balance_usd: usd, balance_cad: usd * rate, snapshot_date: date,
    };
  }),
};

const delta: NetWorthDelta = {
  current_date: date, previous_date: "2026-08-25",
  total: money(total), liquid: money(35_400), invested: money(150_000),
  total_delta: money(8_500), liquid_delta: money(600), invested_delta: money(7_900),
  has_previous: true,
};

const history: NetWorthHistoryPoint[] = [
  124_000, 128_700, 132_100, 138_300, 135_800, 144_500,
  151_200, 157_400, 162_900, 169_800, 176_900, total,
].map((usd, index) => ({
  date: new Date(Date.UTC(2025, 9 + index, 24)).toISOString().slice(0, 10),
  total_usd: usd,
  total_cad: usd * rate,
}));

const goal: GoalProgress = {
  target_usd: 250_000, current_usd: total, gap_usd: 250_000 - total,
  progress: total / 250_000, already_met: false,
  daily_rate_usd: 8_500 / 30, monthly_rate_usd: 8_500, window_days: 30,
  projected_date: "2027-05-11", days_to_goal: 229,
};

const fire: FirePlan = {
  inputs: {
    current_age: 32, annual_expenses_usd: 57_000, swr_pct: 4,
    annual_return_pct: 5, retirement_age: 60, monthly_contribution_usd: 7_250,
  },
  current_usd: total, monthly_contribution_usd: 7_250, contribution_is_derived: false,
  fire_number: 1_425_000, coast_number: 363_502,
  fire_progress: total / 1_425_000, coast_progress: total / 363_502,
  already_fire: false, already_coast: false,
  coast_months: 19, coast_age: 33.6, coast_date: "2028-04-24",
  fire_months: 122, fire_age: 42.2, fire_date: "2036-11-24",
};

const progress: ProgressMetrics = {
  inputs: { base_salary_usd: 95_000, monthly_expenses_usd: 4_750, years_earning: 8 },
  current_usd: total, monthly_expenses_usd: 4_750, expenses_derived: false,
  freedom_months: total / 4_750, freedom_years: total / 57_000,
  salary_multiple: total / 95_000,
  milestones: [1, 2, 3, 5, 10].map((multiple) => ({
    multiple, target_usd: 95_000 * multiple,
    reached: total >= 95_000 * multiple,
    progress: Math.min(1, total / (95_000 * multiple)),
  })),
};

const cashflow: CashflowSummary = {
  window_days: 30, since: "2026-08-25", through: date,
  income: money(12_000), fixed: money(3_200), variable: money(1_550),
  net_savings: money(7_250), net_worth_change: money(8_500),
  investment_growth: money(1_250), savings_rate: 8_500 / 12_000,
  transfer_count: 4, txn_count: 48, currency_warning: false,
  variable_by_category: [
    { category: "Groceries", amount: money(650) },
    { category: "Dining", amount: money(420) },
    { category: "Transport", amount: money(280) },
    { category: "Entertainment", amount: money(200) },
  ],
};

const dividends: DividendSummary = {
  estimated_annual: money(3_140), recorded_last_12_months: money(2_890),
  recorded_payment_count: 36, recorded_period_start: "2025-09-25",
  holdings: [
    { account_id: 1, account_name: "US brokerage", symbol: "VTI", lookup_symbol: "VTI", quantity: 250, annual_dividend_per_share: 4.6, annual_income: 1_150, currency: "USD", estimated_annual: money(1_150), yield_percent: 4.6 / 280 * 100, researched_on: date },
    { account_id: 2, account_name: "Canadian TFSA", symbol: "VDY", lookup_symbol: "VDY.TO", quantity: 600, annual_dividend_per_share: 3.0375, annual_income: 1_822.5, currency: "CAD", estimated_annual: money(1_350), yield_percent: 3.0375 / 80 * 100, researched_on: date },
    { account_id: 4, account_name: "Retirement savings", symbol: "XGRO", lookup_symbol: "XGRO.TO", quantity: 1_080, annual_dividend_per_share: 0.8, annual_income: 864, currency: "CAD", estimated_annual: money(640), yield_percent: 2, researched_on: date },
  ],
  position_count: 3, researched_positions: 3, dividend_positions: 3,
  unresolved_positions: 0, unresolved_symbols: [], stale_positions: 0,
  research_error_count: 0, research_source: "Fictional demo distribution data",
  research_as_of: date, currency_warning: false,
};

let currentValue = total;
let moveValue = total;
const monthlyReturn = Math.pow(1.05, 1 / 12) - 1;
const points = Array.from({ length: 25 }, (_, month) => {
  if (month > 0) {
    currentValue = currentValue * (1 + monthlyReturn) + 7_250;
    moveValue = moveValue * (1 + monthlyReturn) + (month <= 9 ? 7_250 : 3_850);
  }
  return {
    month, date: new Date(Date.UTC(2026, 8 + month, 24)).toISOString().slice(0, 10),
    current_usd: currentValue, seattle_usd: moveValue,
  };
});

const projection: SeattleProjection = {
  start_usd: total, start_date: date, transition_date: "2027-06-24",
  current_monthly_contribution_usd: 7_250, seattle_monthly_contribution_usd: 3_850,
  current_end_usd: currentValue, seattle_end_usd: moveValue,
  end_gap_usd: moveValue - currentValue, points,
  assumptions: {
    current_net_monthly_usd: 12_000, current_expenses_monthly_usd: 4_750,
    seattle_net_monthly_usd: 9_600, seattle_expenses_monthly_usd: 5_750,
    transition_months: 9, horizon_months: 24, annual_return_pct: 5,
  },
};

export const widgetSettings: MacWidgetSettings = {
  platform_supported: true, available: true, enabled: false, unavailable_reason: null,
};

const ai: AiSettings = {
  provider: "ollama", copilot_model: "", ollama_model: "demo",
  ollama_url: "http://localhost:11434", include_real_data: false,
};

export const responses: Record<string, unknown> = {
  list_accounts: accounts,
  get_net_worth: netWorth,
  get_net_worth_delta: delta,
  get_net_worth_history: history,
  get_goal_progress: goal,
  get_fire_plan: fire,
  get_progress_metrics: progress,
  get_cashflow_summary: cashflow,
  get_dividend_summary: dividends,
  get_seattle_projection: projection,
  refresh_fx_rates_if_stale: [],
  refresh_fx_rates: [],
  get_mac_widget_settings: widgetSettings,
  refresh_mac_widget: widgetSettings,
  ai_get_settings: ai,
  ai_list_threads: [],
  "plugin:updater|check": null,
};
