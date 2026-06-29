import { invoke } from "@tauri-apps/api/core";
import type {
  AiChatResponse,
  AiSettings,
  AiSettingsInput,
  ChatMessage,
  ChatThread,
  ModelInfo,
  StoredMessage,
  ToolStep,
} from "../types/ai";
import type {
  Account,
  AddAccountPayload,
  AddBalanceSnapshotPayload,
  BackfillResult,
  CashflowSummary,
  CategorizeResult,
  ClassifiedTransaction,
  FlowType,
  FxRate,
  FireInputs,
  FirePlan,
  GoalProgress,
  ImportPayload,
  ImportSummary,
  NetWorth,
  NetWorthDelta,
  NetWorthHistoryPoint,
  QuestradeStatus,
  QuestradeSyncSummary,
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

/** Override an account's currency (e.g. correct a connector that mislabels a JMD account as CAD). */
export const updateAccountCurrency = (
  accountId: number,
  currency: string,
): Promise<void> => invoke("update_account_currency", { accountId, currency });

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

/** Reconstruct historical balance snapshots from transactions so the chart has a trend to draw. */
export const backfillNetWorthHistory = (): Promise<BackfillResult> =>
  invoke("backfill_net_worth_history");

export const getGoalProgress = (): Promise<GoalProgress> =>
  invoke("get_goal_progress");

/** Update the headline net-worth milestone (USD); returns recomputed progress. */
export const setGoalTarget = (targetUsd: number): Promise<GoalProgress> =>
  invoke("set_goal_target", { targetUsd });

/** Generic FIRE plan: FIRE/CoastFIRE numbers + projected ages from net worth and saved inputs. */
export const getFirePlan = (): Promise<FirePlan> => invoke("get_fire_plan");

/** Persist edited FIRE inputs; returns the recomputed plan. */
export const setFireInputs = (inputs: FireInputs): Promise<FirePlan> =>
  invoke("set_fire_inputs", { inputs });

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

// --- Questrade (direct API) ------------------------------------------------

export const questradeGetStatus = (): Promise<QuestradeStatus> =>
  invoke("questrade_get_status");

/** Exchange a Questrade refresh token and store the rotated token in the local secret store. */
export const questradeConnect = (
  refreshToken: string,
): Promise<QuestradeStatus> =>
  invoke("questrade_connect", { refreshToken });

export const questradeSync = (): Promise<QuestradeSyncSummary> =>
  invoke("questrade_sync");

export const questradeDisconnect = (): Promise<QuestradeStatus> =>
  invoke("questrade_disconnect");

// --- AI advisor ("second brain", Phase 5) ----------------------------------

export const aiGetSettings = (): Promise<AiSettings> =>
  invoke("ai_get_settings");

export const aiSaveSettings = (settings: AiSettingsInput): Promise<AiSettings> =>
  invoke("ai_save_settings", { settings });

/** Store (or clear, with an empty string) the GitHub Models token. Returns whether one is set. */
export const aiSetGithubToken = (token: string): Promise<boolean> =>
  invoke("ai_set_github_token", { token });

/** Reuse the local GitHub CLI session (`gh auth token`) as the GitHub Models token. */
export const aiGithubCliLogin = (): Promise<AiSettings> =>
  invoke("ai_github_cli_login");

/** List models for the active provider (GitHub catalog or local Ollama). */
export const aiListModels = (): Promise<ModelInfo[]> =>
  invoke("ai_list_models");

/** Send the conversation; the advisor calls finance tools on demand and returns a tool trace. */
export const aiChat = (messages: ChatMessage[]): Promise<AiChatResponse> =>
  invoke("ai_chat", { messages });

/** List saved chat threads, most recently active first. */
export const aiListThreads = (): Promise<ChatThread[]> =>
  invoke("ai_list_threads");

/** Create a new (empty) thread; title defaults until the first user message. */
export const aiCreateThread = (title?: string): Promise<ChatThread> =>
  invoke("ai_create_thread", { title });

/** Rename a thread. */
export const aiRenameThread = (threadId: number, title: string): Promise<void> =>
  invoke("ai_rename_thread", { threadId, title });

/** Delete a thread and all of its messages. */
export const aiDeleteThread = (threadId: number): Promise<void> =>
  invoke("ai_delete_thread", { threadId });

/** Load a thread's messages in order, with tool steps rehydrated. */
export const aiGetThreadMessages = (threadId: number): Promise<StoredMessage[]> =>
  invoke("ai_get_thread_messages", { threadId });

/** Append a message to a thread; returns the stored row. */
export const aiAppendMessage = (
  threadId: number,
  role: ChatMessage["role"],
  content: string,
  steps?: ToolStep[],
): Promise<StoredMessage> =>
  invoke("ai_append_message", { threadId, role, content, steps });

/**
 * Ask the configured model to label recent transactions by spending category and flag any that
 * are really internal transfers. Stored categories win over local guesses; transfer flags only
 * touch rows you haven't manually retagged.
 */
export const aiCategorizeTransactions = (
  limit?: number,
): Promise<CategorizeResult> => invoke("ai_categorize_transactions", { limit });

interface BalanceSnapshotResult {
  id: number;
  account_id: number;
  snapshot_date: string;
  balance: number;
  currency: string;
}
