use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

/// The full DDL for TrueNorth's Phase 1 schema.
///
/// Design notes:
/// - Every monetary value carries its currency as a sibling column.
/// - `balance_snapshots` is the time-series backbone for net-worth history.
/// - `fx_rates` stores fetched exchange rates keyed by (from, to, date).
/// - `connector_kind` + `connector_ref` on `accounts` are the hook for Phase 2+ connectors.
/// - Upgrade path: swap `features = ["bundled"]` → `["bundled-sqlcipher"]` in Cargo.toml
///   and call `PRAGMA key = '...'` immediately after opening the connection.
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT    NOT NULL,
    institution      TEXT    NOT NULL,
    account_type     TEXT    NOT NULL,
    currency         TEXT    NOT NULL DEFAULT 'USD',
    jurisdiction     TEXT    NOT NULL DEFAULT 'US',
    connector_kind   TEXT    NOT NULL DEFAULT 'manual',
    connector_ref    TEXT,
    is_active        INTEGER NOT NULL DEFAULT 1,
    notes            TEXT,
    created_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- An ignored binding is a durable tombstone, including after a provider handoff. Absence means
-- unreviewed, never permission to import. account_id is retained when an imported row is hidden.
CREATE TABLE IF NOT EXISTS account_sync_selections (
    provider   TEXT NOT NULL CHECK (provider IN ('snaptrade', 'simplefin')),
    remote_id  TEXT NOT NULL CHECK (length(trim(remote_id)) > 0),
    account_id INTEGER REFERENCES accounts(id),
    decision   TEXT NOT NULL CHECK (decision IN ('sync', 'ignore')),
    institution TEXT,
    connection_id TEXT,
    PRIMARY KEY (provider, remote_id),
    CHECK (decision != 'sync' OR account_id IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_account_sync_owner
    ON account_sync_selections (account_id) WHERE decision = 'sync';

CREATE TABLE IF NOT EXISTS balance_snapshots (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    snapshot_date TEXT    NOT NULL,
    balance       REAL    NOT NULL,
    currency      TEXT    NOT NULL,
    source        TEXT    NOT NULL DEFAULT 'manual',
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (account_id, snapshot_date)
);

CREATE TABLE IF NOT EXISTS fx_rates (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    from_currency TEXT    NOT NULL,
    to_currency   TEXT    NOT NULL,
    rate          REAL    NOT NULL,
    rate_date     TEXT    NOT NULL,
    source        TEXT    NOT NULL DEFAULT 'yahoo',
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (from_currency, to_currency, rate_date)
);

CREATE TABLE IF NOT EXISTS holdings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    symbol        TEXT    NOT NULL,
    quantity      REAL    NOT NULL,
    average_cost  REAL,
    currency      TEXT    NOT NULL,
    last_price    REAL,
    last_price_at TEXT,
    updated_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (account_id, symbol)
);

-- Daily market research that turns current share counts into an annual dividend estimate.
-- `holding_currency` is part of the key because the same bare ticker can refer to a different
-- listing in Canada and the US. Older cached data remains useful during an offline launch.
CREATE TABLE IF NOT EXISTS dividend_research (
    symbol                     TEXT NOT NULL,
    holding_currency           TEXT NOT NULL,
    lookup_symbol              TEXT NOT NULL,
    quote_currency             TEXT NOT NULL,
    annual_dividend_per_share  REAL NOT NULL,
    current_price              REAL,
    researched_on              TEXT NOT NULL,
    source                     TEXT NOT NULL DEFAULT 'yahoo',
    PRIMARY KEY (symbol, holding_currency)
);

CREATE TABLE IF NOT EXISTS transactions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    txn_date      TEXT    NOT NULL,
    description   TEXT    NOT NULL,
    amount        REAL    NOT NULL,
    currency      TEXT    NOT NULL,
    category      TEXT,
    memo          TEXT,
    connector_ref TEXT,
    -- Manual fixed/variable/income/transfer override; wins over rule-based classification and
    -- is preserved across re-syncs (the connector upsert never touches this column).
    flow_override TEXT,
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Rules that auto-classify a transaction by case-insensitive substring of its description.
-- flow_type is one of 'income' | 'fixed' | 'variable' | 'transfer'. Earlier rows win.
CREATE TABLE IF NOT EXISTS txn_rules (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern    TEXT NOT NULL,
    flow_type  TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS goals (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT    NOT NULL,
    target_amount       REAL    NOT NULL,
    currency            TEXT    NOT NULL DEFAULT 'CAD',
    target_date         TEXT,
    linked_account_ids  TEXT,
    notes               TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- AI advisor chat history. A thread groups an ongoing conversation so context is retained
-- across app restarts; messages cascade-delete with their thread.
CREATE TABLE IF NOT EXISTS chat_threads (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT    NOT NULL DEFAULT 'New chat',
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id  INTEGER NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    role       TEXT    NOT NULL,
    content    TEXT    NOT NULL,
    -- JSON array of tool-call steps for assistant turns; NULL for user turns.
    steps_json TEXT,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Performance indices
CREATE INDEX IF NOT EXISTS idx_balance_snapshots_account_date
    ON balance_snapshots (account_id, snapshot_date DESC);

CREATE INDEX IF NOT EXISTS idx_fx_rates_pair_date
    ON fx_rates (from_currency, to_currency, rate_date DESC);

CREATE INDEX IF NOT EXISTS idx_transactions_account_date
    ON transactions (account_id, txn_date DESC);

CREATE INDEX IF NOT EXISTS idx_chat_messages_thread
    ON chat_messages (thread_id, id);

-- Dedup key for connector-sourced transactions. connector_ref is NULL for manual rows, and
-- SQLite treats NULLs as distinct, so manual entries never collide on this index.
CREATE UNIQUE INDEX IF NOT EXISTS idx_transactions_connector
    ON transactions (account_id, connector_ref);
"#;

/// Apply the schema DDL and ensure WAL mode for better concurrency.
pub fn apply_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(SCHEMA)?;
    // Lightweight migrations for databases created before a column existed. CREATE TABLE
    // IF NOT EXISTS never alters an existing table, so additive columns are added here.
    add_column_if_missing(conn, "transactions", "flow_override", "TEXT")?;
    // Preserve existing imported identities, not guesses based on names or balances. Ambiguous
    // legacy IDs are left unmapped and explicitly rejected by the account-review/sync commands.
    conn.execute(
        "INSERT INTO account_sync_selections (provider, remote_id, account_id, decision, institution) \
         SELECT connector_kind, connector_ref, id, \
                CASE WHEN is_active = 1 THEN 'sync' ELSE 'ignore' END, institution \
         FROM accounts \
         WHERE connector_kind IN ('snaptrade', 'simplefin') \
           AND connector_ref IS NOT NULL AND length(trim(connector_ref)) > 0 \
         GROUP BY connector_kind, connector_ref HAVING COUNT(*) = 1 \
         ON CONFLICT(provider, remote_id) DO NOTHING",
        [],
    )?;
    Ok(())
}

/// Normalize aggregator-managed Questrade accounts and suppress them while direct Questrade data is
/// active. This runs on launch and after every relevant connector sync so ordering cannot revive a
/// redundant account or restore an old USD-derived US jurisdiction.
pub fn reconcile_aggregated_questrade_accounts(conn: &Connection) -> SqlResult<usize> {
    conn.execute(
        "UPDATE accounts SET jurisdiction = 'CA', \
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE connector_kind IN ('simplefin', 'snaptrade') \
         AND (lower(institution) LIKE '%questrade%' OR EXISTS ( \
             SELECT 1 FROM account_sync_selections s WHERE s.account_id = accounts.id \
             AND s.provider = accounts.connector_kind AND s.remote_id = accounts.connector_ref \
             AND lower(s.institution) LIKE '%questrade%')) AND jurisdiction != 'CA'",
        [],
    )?;

    conn.execute(
        "UPDATE accounts SET is_active = 0, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE is_active = 1 AND connector_kind IN ('simplefin', 'snaptrade') \
         AND (lower(institution) LIKE '%questrade%' OR EXISTS ( \
             SELECT 1 FROM account_sync_selections s WHERE s.account_id = accounts.id \
             AND s.provider = accounts.connector_kind AND s.remote_id = accounts.connector_ref \
             AND lower(s.institution) LIKE '%questrade%')) \
         AND EXISTS (SELECT 1 FROM accounts AS direct \
                     WHERE direct.connector_kind = 'questrade' AND direct.is_active = 1)",
        [],
    )
}

/// Add `column` to `table` when it isn't already present. Idempotent: a no-op once the column
/// exists, so it's safe to run on every launch.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> SqlResult<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Default transaction-classification rules. Earlier entries win, so specific payees (the mom
/// support transfer) precede the generic transfer patterns that would otherwise swallow them.
/// `transfer` rows are excluded from income/expense so internal moves and card payments don't
/// double-count; the user can edit or delete any of these.
const DEFAULT_TXN_RULES: &[(&str, &str)] = &[
    // The $800/mo support sent to mom is a real fixed expense, not lifestyle creep — and not an
    // internal transfer. Rename the pattern to the exact payee your bank reports if needed.
    ("mom", "fixed"),
    ("rent", "fixed"),
    // Credit-card payments and account-to-account moves: not spending, not income.
    ("payment - thank you", "transfer"),
    ("payment thank you", "transfer"),
    ("bill payment", "transfer"),
    ("e-transfer", "transfer"),
    ("transfer", "transfer"),
];

/// Brokerage, investment, and crypto payees plus money-movement phrases that signal an internal
/// move rather than spending. Money sent to your own brokerage/exchange shouldn't count as
/// variable lifestyle spending. These ship to new installs via [`seed_txn_rules`] and are
/// back-filled into existing installs once via [`seed_txn_rules_v2`]. Kept to distinctive payee
/// substrings to avoid false positives (e.g. no bare "wire", which would catch "wireless").
const BROKERAGE_TRANSFER_RULES: &[(&str, &str)] = &[
    ("wealthsimple", "transfer"),
    ("questrade", "transfer"),
    ("qtrade", "transfer"),
    ("rbc direct investing", "transfer"),
    ("td direct investing", "transfer"),
    ("td waterhouse", "transfer"),
    ("bmo investorline", "transfer"),
    ("cibc investor", "transfer"),
    ("national bank direct", "transfer"),
    ("scotia itrade", "transfer"),
    ("robinhood", "transfer"),
    ("interactive brokers", "transfer"),
    ("charles schwab", "transfer"),
    ("schwab", "transfer"),
    ("fidelity", "transfer"),
    ("vanguard", "transfer"),
    ("e*trade", "transfer"),
    ("etrade", "transfer"),
    ("merrill", "transfer"),
    ("coinbase", "transfer"),
    ("kraken", "transfer"),
    ("crypto.com", "transfer"),
    ("binance", "transfer"),
    ("shakepay", "transfer"),
    ("wealthfront", "transfer"),
    ("betterment", "transfer"),
    ("brokerage", "transfer"),
];

/// Seed the reference account types into app_settings if not already present.
pub fn seed_defaults(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES (?1, ?2)",
        params!["home_currency", "CAD"],
    )?;
    // The headline "master net worth" milestone, in USD (the benchmark currency). Surfaced by the
    // $100k countdown; editable via set_goal_target.
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES (?1, ?2)",
        params!["goal_target_usd", "100000"],
    )?;
    seed_txn_rules(conn)?;
    seed_txn_rules_v2(conn)?;
    purge_inferred_snapshots(conn)?;
    reconcile_aggregated_questrade_accounts(conn)?;
    Ok(())
}

/// Remove reconstructed balance snapshots left over from the retired "reconstruct from
/// transactions" feature (rows stamped `source = 'backfill'`). Net-worth history is now driven
/// only by observed balances, so any inferred rows are cleared on launch to keep the trend — and
/// the delta computation, which reads every snapshot — grounded in real data. Idempotent: once the
/// rows are gone (and nothing writes new ones) this is a cheap no-op.
fn purge_inferred_snapshots(conn: &Connection) -> SqlResult<()> {
    conn.execute("DELETE FROM balance_snapshots WHERE source = 'backfill'", [])?;
    Ok(())
}

/// Insert the default classification rules exactly once. Guarded by a flag so deleting a seeded
/// rule doesn't resurrect it on the next launch.
fn seed_txn_rules(conn: &Connection) -> SqlResult<()> {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'txn_rules_seeded'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if already.is_some() {
        return Ok(());
    }
    for (pattern, flow_type) in DEFAULT_TXN_RULES {
        conn.execute(
            "INSERT INTO txn_rules (pattern, flow_type) VALUES (?1, ?2)",
            params![pattern, flow_type],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES ('txn_rules_seeded', '1')",
        [],
    )?;
    Ok(())
}

/// Back-fill the brokerage/transfer rules into installs that were seeded before they existed.
/// Guarded by its own flag so it runs once; skips any pattern the user already has so we never
/// create duplicates. New installs hit this too (right after [`seed_txn_rules`]), which is how
/// they receive [`BROKERAGE_TRANSFER_RULES`].
fn seed_txn_rules_v2(conn: &Connection) -> SqlResult<()> {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'txn_rules_seeded_v2'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if already.is_some() {
        return Ok(());
    }
    for (pattern, flow_type) in BROKERAGE_TRANSFER_RULES {
        conn.execute(
            "INSERT INTO txn_rules (pattern, flow_type) \
             SELECT ?1, ?2 \
             WHERE NOT EXISTS (SELECT 1 FROM txn_rules WHERE lower(pattern) = lower(?1))",
            params![pattern, flow_type],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES ('txn_rules_seeded_v2', '1')",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();
        conn
    }

    #[test]
    fn schema_applies_cleanly() {
        let conn = open_test_db();
        // Idempotent — applying twice must not fail
        apply_schema(&conn).unwrap();
    }

    #[test]
    fn seed_defaults_reconciles_aggregated_questrade_accounts() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
             connector_kind, connector_ref) \
             VALUES ('TFSA', 'Questrade', 'tfsa', 'USD', 'US', 'simplefin', 'sf-1')",
            [],
        )
        .unwrap();

        seed_defaults(&conn).unwrap();
        let (jurisdiction, is_active): (String, i64) = conn
            .query_row(
                "SELECT jurisdiction, is_active FROM accounts WHERE connector_kind = 'simplefin'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(jurisdiction, "CA");
        assert_eq!(is_active, 1);

        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
             connector_kind, connector_ref) \
             VALUES ('TFSA direct', 'Questrade', 'tfsa', 'CAD', 'CA', 'questrade', 'qt-1')",
            [],
        )
        .unwrap();
        seed_defaults(&conn).unwrap();

        let aggregator_active: i64 = conn
            .query_row(
                "SELECT is_active FROM accounts WHERE connector_kind = 'simplefin'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(aggregator_active, 0);
    }

    #[test]
    fn seed_defaults_purges_inferred_backfill_snapshots() {
        // A database upgraded from a version that reconstructed history still holds `backfill`
        // rows. seed_defaults clears them (idempotently) while leaving observed snapshots intact.
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Chase Checking', 'Chase', 'chequing', 'USD', 'US')",
            [],
        )
        .unwrap();
        let account_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO balance_snapshots (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, '2025-01-01', 100.0, 'USD', 'backfill'), \
                    (?1, '2025-02-01', 200.0, 'USD', 'manual')",
            params![account_id],
        )
        .unwrap();

        seed_defaults(&conn).unwrap();

        let inferred: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots WHERE source = 'backfill'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inferred, 0);
        let real: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots WHERE source = 'manual'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(real, 1);
    }

    #[test]
    fn can_insert_and_query_account() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Chase Checking', 'Chase', 'chequing', 'USD', 'US')",
            [],
        )
        .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn balance_snapshot_upsert_works() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Test', 'Test', 'savings', 'CAD', 'CA')",
            [],
        )
        .unwrap();
        let account_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency) VALUES (?1, '2025-01-01', 1000.0, 'CAD')",
            params![account_id],
        )
        .unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency) VALUES (?1, '2025-01-01', 2000.0, 'CAD')",
            params![account_id],
        )
        .unwrap();

        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 2000.0);
    }

    #[test]
    fn seeds_default_txn_rules_once() {
        let conn = open_test_db();
        let total = (DEFAULT_TXN_RULES.len() + BROKERAGE_TRANSFER_RULES.len()) as i64;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM txn_rules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, total);
        // The mom support transfer is seeded as a fixed expense, ahead of the generic
        // transfer rules so it isn't excluded as an internal move.
        let mom: String = conn
            .query_row(
                "SELECT flow_type FROM txn_rules WHERE pattern = 'mom'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mom, "fixed");
        // Brokerage payees are seeded as transfers so funding a brokerage isn't variable spend.
        let ws: String = conn
            .query_row(
                "SELECT flow_type FROM txn_rules WHERE pattern = 'wealthsimple'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ws, "transfer");

        // Re-seeding is a no-op (deleting a rule must not resurrect it).
        conn.execute("DELETE FROM txn_rules WHERE pattern = 'mom'", [])
            .unwrap();
        seed_defaults(&conn).unwrap();
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM txn_rules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, total - 1);
    }

    #[test]
    fn back_fills_brokerage_rules_into_legacy_install() {
        // Simulate an install seeded before the brokerage rules existed: v1 ran, v2 did not.
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_txn_rules(&conn).unwrap();
        // The user had already added their own Wealthsimple rule by hand.
        conn.execute(
            "INSERT INTO txn_rules (pattern, flow_type) VALUES ('wealthsimple', 'transfer')",
            [],
        )
        .unwrap();
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM txn_rules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, DEFAULT_TXN_RULES.len() as i64 + 1);

        // The back-fill adds every brokerage rule except the one they already had.
        seed_txn_rules_v2(&conn).unwrap();
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM txn_rules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            after,
            DEFAULT_TXN_RULES.len() as i64 + BROKERAGE_TRANSFER_RULES.len() as i64
        );
        // No duplicate Wealthsimple rule was created.
        let ws_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM txn_rules WHERE lower(pattern) = 'wealthsimple'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ws_count, 1);

        // Running again is a no-op.
        seed_txn_rules_v2(&conn).unwrap();
        let again: i64 = conn
            .query_row("SELECT COUNT(*) FROM txn_rules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(again, after);
    }

    #[test]
    fn migrates_flow_override_onto_legacy_transactions() {
        // Simulate a pre-tagging database: the transactions table without flow_override.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transactions (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id INTEGER NOT NULL, \
                txn_date TEXT NOT NULL, description TEXT NOT NULL, amount REAL NOT NULL, \
                currency TEXT NOT NULL, category TEXT, memo TEXT, connector_ref TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')));",
        )
        .unwrap();

        apply_schema(&conn).unwrap();
        let has_column = conn
            .prepare("PRAGMA table_info(transactions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .any(|name| name == "flow_override");
        assert!(has_column);

        // Idempotent: running the migration again must not error.
        apply_schema(&conn).unwrap();
    }
}
