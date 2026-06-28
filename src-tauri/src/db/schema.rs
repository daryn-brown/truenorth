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
    Ok(())
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
