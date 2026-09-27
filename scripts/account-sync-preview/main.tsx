import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Dashboard from "../../src/pages/Dashboard";
import type {
  Account, AccountSyncChoice, AccountSyncReview, DiscoveredSyncAccount,
  SaveAccountSyncChoices, SyncProvider,
} from "../../src/types/finance";
import { responses } from "../readme-preview/fixtures";
import "../../src/index.css";
import "../../src/apps/desktop/desktop.css";

if (!import.meta.env.DEV) throw new Error("Account sync fixtures are development-only.");

const stamp = "2026-09-27T12:00:00Z";
const accountRows = [
  { id: 1, name: "Individual (0580)", connector_kind: "simplefin", connector_ref: "sf-1", account_type: "brokerage" },
  { id: 2, name: "Individual (7986)", connector_kind: "manual", connector_ref: null, account_type: "brokerage" },
  { id: 3, name: "Roth IRA (6755)", connector_kind: "simplefin", connector_ref: "sf-3", account_type: "roth_ira" },
  { id: 4, name: "Roth duplicate", connector_kind: "snaptrade", connector_ref: "snap-3", account_type: "roth_ira" },
] satisfies Pick<Account, "id" | "name" | "connector_kind" | "connector_ref" | "account_type">[];

const accounts: Account[] = accountRows.map((row) => ({
  ...row,
  institution: "Robinhood", currency: "USD", jurisdiction: "US",
  is_active: true, notes: "Fictional regression fixture",
  created_at: stamp, updated_at: stamp, latest_balance: 1000, latest_balance_date: "2026-09-27",
} satisfies Account));

function remote(
  id: string,
  name: string,
  number: string,
  accountType: DiscoveredSyncAccount["account_type"],
  localId: number | null = null,
): DiscoveredSyncAccount {
  return {
    remote_id: id, connection_id: null, name, institution: "Robinhood", account_type: accountType,
    currency: "USD", masked_number: `****${number}`,
    selection: localId === null ? "unreviewed" : "sync", local_account_id: localId,
    can_resume: localId !== null, unavailable_reason: null, linkable_account_ids: [],
  };
}

const remotes: Record<SyncProvider, DiscoveredSyncAccount[]> = {
  snaptrade: [
    remote("snap-1", "Individual", "0580", "brokerage"),
    remote("snap-2", "Individual", "7986", "brokerage"),
    remote("snap-3", "Roth IRA", "6755", "roth_ira", 4),
    { ...remote("snap-qt", "TFSA", "1111", "tfsa"), institution: "Questrade", currency: "CAD" },
  ],
  simplefin: [
    remote("sf-1", "Individual", "0580", "brokerage", 1),
    remote("sf-3", "Roth IRA", "6755", "roth_ira", 3),
    remote("sf-unreviewed", "New savings", "8888", "savings"),
  ],
};

interface Fixture {
  calls: { command: string; payload: unknown }[];
  failCommand: string | null;
  holdDelete: boolean;
  releaseDelete: (() => void) | null;
  automaticCheckDue: boolean;
}

declare global {
  interface Window { accountSyncFixture: Fixture; }
}

const fixture: Fixture = {
  calls: [], failCommand: null, holdDelete: false, releaseDelete: null,
  automaticCheckDue: new URLSearchParams(location.search).has("automatic"),
};
window.accountSyncFixture = fixture;
let revision = 0;
const connected: Record<SyncProvider, boolean> = { snaptrade: true, simplefin: true };

function review(provider: SyncProvider): AccountSyncReview {
  return structuredClone({
    revision: String(revision),
    accounts: remotes[provider].map((r) => ({
      ...r,
      linkable_account_ids: r.local_account_id === null ? accounts.filter((a) =>
        a.is_active && a.currency === r.currency && a.account_type === r.account_type
        && a.connector_kind !== provider,
      ).map((a) => a.id) : [],
    })),
    local_accounts: accounts,
    warnings: [],
  });
}

function isChoice(value: unknown): value is AccountSyncChoice {
  if (!value || typeof value !== "object" || !("remote_id" in value) || typeof value.remote_id !== "string"
      || !("action" in value)) return false;
  return value.action === "create" || value.action === "sync" || value.action === "ignore"
    || (value.action === "link" && "account_id" in value && Number.isSafeInteger(value.account_id));
}

function isSave(value: unknown): value is SaveAccountSyncChoices {
  return Boolean(value && typeof value === "object"
    && "revision" in value && typeof value.revision === "string"
    && "choices" in value && Array.isArray(value.choices) && value.choices.every(isChoice));
}

function saveChoices(provider: SyncProvider, payload: SaveAccountSyncChoices) {
  if (payload.revision !== String(revision)) throw new Error("Account choices changed. Reload the review.");
  for (const choice of payload.choices) {
    const row = remotes[provider].find((r) => r.remote_id === choice.remote_id);
    if (!row) throw new Error("Unknown fictional account.");
    if (choice.action === "ignore") {
      row.selection = "ignore";
      const account = accounts.find((a) => a.id === row.local_account_id);
      if (account?.connector_kind === provider && account.connector_ref === row.remote_id) account.is_active = false;
    } else {
      if (choice.action === "create") {
        const id = Math.max(...accounts.map((a) => a.id)) + 1;
        accounts.push({
          id, name: row.name, institution: row.institution, account_type: row.account_type,
          currency: row.currency || "USD", jurisdiction: row.currency === "CAD" ? "CA" : "US",
          connector_kind: provider, connector_ref: row.remote_id, is_active: true,
          notes: null, created_at: stamp, updated_at: stamp,
        });
        row.local_account_id = id;
      } else if (choice.action === "link") {
        row.local_account_id = choice.account_id;
      }
      const account = accounts.find((a) => a.id === row.local_account_id);
      if (!account) throw new Error("Unknown fictional target.");
      for (const otherProvider of ["snaptrade", "simplefin"] as const) {
        for (const other of remotes[otherProvider]) {
          if (other.local_account_id === account.id) other.selection = "ignore";
        }
      }
      account.connector_kind = provider;
      account.connector_ref = row.remote_id;
      account.is_active = true;
      row.selection = "sync";
      row.can_resume = true;
    }
  }
  revision++;
  return review(provider);
}

mockIPC(async (command, payload) => {
  fixture.calls.push({ command, payload: structuredClone(payload) });
  if (fixture.failCommand === command) {
    fixture.failCommand = null;
    throw new Error(`Synthetic failure for ${command}`);
  }
  if (command === "list_accounts") return structuredClone(accounts.filter((a) => a.is_active));
  if (command === "questrade_get_status") {
    return { is_connected: false, last_synced_at: null, account_count: 0 };
  }
  if (command === "plugin:shell|open") return;
  if (command === "delete_account") {
    if (!payload || !("accountId" in payload) || typeof payload.accountId !== "number") {
      throw new Error("Delete requires an accountId.");
    }
    if (fixture.holdDelete) {
      await new Promise<void>((resolve) => { fixture.releaseDelete = resolve; });
      fixture.holdDelete = false;
      fixture.releaseDelete = null;
    }
    const account = accounts.find((a) => a.id === payload.accountId);
    if (!account) throw new Error("Account not found.");
    account.is_active = false;
    for (const provider of ["snaptrade", "simplefin"] as const) {
      for (const row of remotes[provider]) {
        if (row.local_account_id === account.id) row.selection = "ignore";
      }
    }
    revision++;
    return;
  }
  const provider: SyncProvider = command.startsWith("snaptrade") ? "snaptrade" : "simplefin";
  if (command === `${provider}_get_status`) {
    return {
      has_credentials: true, is_connected: connected[provider], is_personal: true,
      client_id: "PERS-SYNTHETIC", last_synced_at: null,
      account_count: accounts.filter((a) => a.is_active && a.connector_kind === provider).length,
      last_attempt_at: fixture.automaticCheckDue ? null : new Date().toISOString(),
      app_auth_required: false, messages: [],
      connections: provider === "simplefin" ? [{
        id: "sf-bank", name: "Robinhood", status: "stale", messages: ["Synthetic stale balance"],
        accounts: accounts.filter((a) => a.is_active && a.connector_kind === provider).map((a) => ({
          account_id: a.id, name: a.name,
          health: { status: "stale", balance_as_of: stamp, message: "Synthetic stale balance" },
        })),
      }].filter((b) => b.accounts.length > 0) : [],
    };
  }
  if (command === `${provider}_discover_accounts`) return review(provider);
  if (command === `${provider}_save_account_choices`) {
    if (!payload || !("payload" in payload) || !isSave(payload.payload)) throw new Error("Invalid selection payload.");
    return saveChoices(provider, payload.payload);
  }
  if (command === `${provider}_sync`) {
    fixture.automaticCheckDue = false;
    const selected = remotes[provider].filter((r) => r.selection === "sync");
    if (selected.length === 0) throw new Error("No accounts are selected for sync. Open Choose accounts to sync.");
    return {
      accounts_synced: selected.length, holdings_synced: selected.length, transactions_synced: 0,
      synced_at: stamp, warnings: [], skipped: false,
      accounts_needing_review: remotes[provider].filter((r) => r.selection === "unreviewed").length,
    };
  }
  if (command === `${provider}_disconnect`) {
    connected[provider] = false;
    for (const r of remotes[provider]) r.selection = "ignore";
    for (const a of accounts) if (a.connector_kind === provider) a.is_active = false;
    revision++;
    return {
      has_credentials: true, is_connected: false, is_personal: true,
      client_id: "PERS-SYNTHETIC", last_synced_at: null, account_count: 0,
      last_attempt_at: null, app_auth_required: false, messages: [], connections: [],
    };
  }
  if (Object.prototype.hasOwnProperty.call(responses, command)) return structuredClone(responses[command]);
  throw new Error(`Unsupported fictional IPC command: ${command}`);
}, { shouldMockEvents: true });

const root = document.getElementById("root");
if (!root) throw new Error("Missing fixture root.");
createRoot(root).render(<div className="desktop-app"><Dashboard /></div>);
