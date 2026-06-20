use rusqlite::{params, Connection, Result as SqlResult};

/// The full DDL for Finance Second Brain's Phase 1 schema.
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
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
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

-- Performance indices
CREATE INDEX IF NOT EXISTS idx_balance_snapshots_account_date
    ON balance_snapshots (account_id, snapshot_date DESC);

CREATE INDEX IF NOT EXISTS idx_fx_rates_pair_date
    ON fx_rates (from_currency, to_currency, rate_date DESC);

CREATE INDEX IF NOT EXISTS idx_transactions_account_date
    ON transactions (account_id, txn_date DESC);
"#;

/// Apply the schema DDL and ensure WAL mode for better concurrency.
pub fn apply_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Seed the reference account types into app_settings if not already present.
pub fn seed_defaults(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO app_settings (key, value) VALUES (?1, ?2)",
        params!["home_currency", "CAD"],
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
}
