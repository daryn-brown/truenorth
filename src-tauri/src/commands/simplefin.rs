//! SimpleFIN Tauri commands: claim a setup token, sync balances + holdings, disconnect.
//!
//! SimpleFIN needs no signing or user registration — the claimed **access URL** (stored in the
//! keychain) is all that's required. As with SnapTrade, the SQLite mutex is never held across an
//! `.await`: network calls happen first, then results are written under a short-lived lock, and
//! one balance snapshot per account flows straight into the existing net-worth pipeline.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::connector::simplefin::{
    claim_access_url, SimpleFinAccount, SimpleFinAccountSet, SimpleFinClient, SimpleFinError,
    SimpleFinHolding,
};
use crate::db::secrets::{self, SIMPLEFIN_ACCESS_URL};
use crate::db::AppDb;

const SETTING_LAST_SYNCED: &str = "simplefin_last_synced_at";

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SimpleFinStatus {
    /// An access URL is stored (the user has claimed a setup token).
    pub is_connected: bool,
    pub last_synced_at: Option<String>,
    /// Number of active accounts connected via SimpleFIN.
    pub account_count: i64,
}

#[derive(Debug, Serialize)]
pub struct SimpleFinSyncSummary {
    pub accounts_synced: usize,
    pub holdings_synced: usize,
    pub synced_at: String,
    /// Non-fatal messages SimpleFIN returned (e.g. one institution needs to be re-authenticated).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// app_settings helpers
// ---------------------------------------------------------------------------

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) \
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value],
    )?;
    Ok(())
}

fn delete_setting(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

/// Best-effort map from a SimpleFIN account name (banks rarely report a type) to one of
/// TrueNorth's account-type ids. Accounts that report holdings are treated as brokerage.
fn map_account_type(name: &str, has_holdings: bool) -> String {
    let hay = name.to_uppercase();
    let has = |needle: &str| hay.contains(needle);

    let kind = if has("ROTH") {
        "roth_ira"
    } else if has("401") {
        "401k"
    } else if has("RRSP") {
        "rrsp"
    } else if has("TFSA") {
        "tfsa"
    } else if has("FHSA") {
        "fhsa"
    } else if has("IRA") {
        "ira"
    } else if has("CREDIT") || has("VISA") || has("MASTERCARD") || has("CARD") {
        "credit"
    } else if has("CHEQUING") || has("CHECKING") {
        "chequing"
    } else if has("SAVING") {
        "savings"
    } else if has("CRYPTO") {
        "crypto"
    } else if has_holdings || has("BROKERAGE") || has("INVEST") {
        "brokerage"
    } else {
        // SimpleFIN is bank-focused; default a plain deposit account to chequing.
        "chequing"
    };
    kind.to_string()
}

/// Map an account currency to the jurisdiction the rest of the app reasons about.
fn jurisdiction_for(currency: &str) -> &'static str {
    if currency.eq_ignore_ascii_case("CAD") {
        "CA"
    } else {
        "US"
    }
}

/// Turn a SimpleFIN error into a user-facing message.
fn friendly(e: SimpleFinError) -> String {
    if e.is_auth() {
        "SimpleFIN rejected the access URL. Reconnect with a new setup token from your SimpleFIN \
         bridge (and disable the old one if you think it was exposed)."
            .into()
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// DB reconcile helpers (synchronous — never run while awaiting)
// ---------------------------------------------------------------------------

/// Upsert one SimpleFIN account (keyed by `connector_ref`) and write today's balance snapshot,
/// which the net-worth pipeline picks up automatically. Returns the local account row id.
fn upsert_account(
    conn: &Connection,
    account: &SimpleFinAccount,
    today: &str,
    now: &str,
) -> rusqlite::Result<i64> {
    let reported_currency = &account.currency;
    let jurisdiction = jurisdiction_for(reported_currency);
    let account_type = map_account_type(&account.name, !account.holdings.is_empty());
    let institution = account
        .institution
        .clone()
        .unwrap_or_else(|| "SimpleFIN".to_string());

    // Keyed by connector_ref. On an existing account we deliberately do NOT overwrite the stored
    // currency/jurisdiction: aggregators sometimes mislabel a foreign account's currency (e.g.
    // SimpleFIN reporting a Jamaican JMD account as CAD). The user can correct it via
    // `update_account_currency`, and preserving the stored value keeps that fix across syncs.
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, currency FROM accounts WHERE connector_kind = 'simplefin' AND connector_ref = ?1",
            params![account.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let (account_id, currency) = if let Some((id, stored_currency)) = existing {
        conn.execute(
            "UPDATE accounts SET name = ?1, institution = ?2, account_type = ?3, \
             is_active = 1, updated_at = ?4 WHERE id = ?5",
            params![account.name, institution, account_type, now, id],
        )?;
        (id, stored_currency)
    } else {
        conn.execute(
            "INSERT INTO accounts \
             (name, institution, account_type, currency, jurisdiction, connector_kind, connector_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'simplefin', ?6)",
            params![account.name, institution, account_type, reported_currency, jurisdiction, account.id],
        )?;
        (conn.last_insert_rowid(), reported_currency.clone())
    };

    if let Some(total) = account.balance {
        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, ?2, ?3, ?4, 'simplefin')",
            params![account_id, today, total, currency],
        )?;
    }

    Ok(account_id)
}

/// SimpleFIN reports `market_value`/`cost_basis` as position totals; derive the per-share
/// `(last_price, average_cost)` the holdings table stores. Guards against a zero-share divide.
fn holding_unit_prices(h: &SimpleFinHolding) -> (Option<f64>, Option<f64>) {
    if h.shares != 0.0 {
        (
            h.market_value.map(|mv| mv / h.shares),
            h.cost_basis.map(|cb| cb / h.shares),
        )
    } else {
        (None, None)
    }
}

/// Replace an account's holdings so closed positions disappear. Returns the number inserted.
fn replace_holdings(
    conn: &Connection,
    account_id: i64,
    account: &SimpleFinAccount,
    now: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM holdings WHERE account_id = ?1",
        params![account_id],
    )?;
    let mut count = 0usize;
    for h in &account.holdings {
        let (last_price, average_cost) = holding_unit_prices(h);
        let holding_currency = h.currency.clone().unwrap_or_else(|| account.currency.clone());
        let last_price_at = last_price.map(|_| now.to_string());
        conn.execute(
            "INSERT OR REPLACE INTO holdings \
             (account_id, symbol, quantity, average_cost, currency, last_price, last_price_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                h.symbol,
                h.shares,
                average_cost,
                holding_currency,
                last_price,
                last_price_at,
                now
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Report whether SimpleFIN is connected, plus last-synced time and connected-account count.
#[tauri::command]
pub fn simplefin_get_status(db: State<AppDb>) -> Result<SimpleFinStatus, String> {
    let (last_synced_at, account_count) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let last_synced_at = get_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        let account_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'simplefin' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        (last_synced_at, account_count)
    };

    let access_url = secrets::get_secret(SIMPLEFIN_ACCESS_URL).map_err(|e| e.to_string())?;

    Ok(SimpleFinStatus {
        is_connected: access_url.is_some(),
        last_synced_at,
        account_count,
    })
}

/// Claim a SimpleFIN setup token, validate the resulting access URL, and store it in the keychain.
#[tauri::command]
pub async fn simplefin_connect(
    db: State<'_, AppDb>,
    setup_token: String,
) -> Result<SimpleFinStatus, String> {
    let setup_token = setup_token.trim().to_string();
    if setup_token.is_empty() {
        return Err("Paste the setup token from your SimpleFIN bridge first.".into());
    }

    // Exchange the one-time token for a durable access URL, then confirm it actually works.
    let access_url = claim_access_url(&setup_token).await.map_err(friendly)?;
    SimpleFinClient::new(access_url.clone())
        .check()
        .await
        .map_err(friendly)?;

    secrets::set_secret(SIMPLEFIN_ACCESS_URL, &access_url).map_err(|e| e.to_string())?;

    simplefin_get_status(db)
}

/// Pull accounts + balances + holdings from SimpleFIN and reconcile them into the local DB.
/// Writes one balance snapshot per account so net worth updates automatically.
#[tauri::command]
pub async fn simplefin_sync(db: State<'_, AppDb>) -> Result<SimpleFinSyncSummary, String> {
    let access_url = secrets::get_secret(SIMPLEFIN_ACCESS_URL)
        .map_err(|e| e.to_string())?
        .ok_or("Connect SimpleFIN before syncing.")?;

    // Fetch over the network first — no DB lock is held across an await.
    let account_set: SimpleFinAccountSet = SimpleFinClient::new(access_url)
        .fetch_accounts()
        .await
        .map_err(friendly)?;

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut accounts_synced = 0usize;
    let mut holdings_synced = 0usize;

    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        for account in &account_set.accounts {
            let account_id =
                upsert_account(&tx, account, &today, &now).map_err(|e| e.to_string())?;
            accounts_synced += 1;
            holdings_synced +=
                replace_holdings(&tx, account_id, account, &now).map_err(|e| e.to_string())?;
        }

        set_setting(&tx, SETTING_LAST_SYNCED, &now).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(SimpleFinSyncSummary {
        accounts_synced,
        holdings_synced,
        synced_at: now,
        warnings: account_set.errors,
    })
}

/// Disconnect SimpleFIN: remove the stored access URL, clear the last-synced marker, and
/// deactivate the connected accounts. Historical snapshots are left untouched.
#[tauri::command]
pub fn simplefin_disconnect(db: State<AppDb>) -> Result<SimpleFinStatus, String> {
    secrets::delete_secret(SIMPLEFIN_ACCESS_URL).map_err(|e| e.to_string())?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        delete_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE connector_kind = 'simplefin'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    simplefin_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_account_types_from_name_and_holdings() {
        assert_eq!(map_account_type("TFSA Investment", true), "tfsa");
        assert_eq!(map_account_type("Roth IRA", false), "roth_ira");
        assert_eq!(map_account_type("Everyday Chequing", false), "chequing");
        assert_eq!(map_account_type("High-Interest Savings", false), "savings");
        assert_eq!(map_account_type("Visa Platinum", false), "credit");
        assert_eq!(map_account_type("Self-Directed", true), "brokerage");
        // A plain deposit account with no hints defaults to chequing.
        assert_eq!(map_account_type("My Account", false), "chequing");
    }

    #[test]
    fn jurisdiction_follows_currency() {
        assert_eq!(jurisdiction_for("CAD"), "CA");
        assert_eq!(jurisdiction_for("cad"), "CA");
        assert_eq!(jurisdiction_for("USD"), "US");
    }

    #[test]
    fn holding_unit_prices_divide_totals_and_guard_zero_shares() {
        let h = SimpleFinHolding {
            symbol: "AAPL".into(),
            shares: 10.0,
            market_value: Some(1500.0),
            cost_basis: Some(1000.0),
            currency: Some("USD".into()),
        };
        assert_eq!(holding_unit_prices(&h), (Some(150.0), Some(100.0)));

        let zero = SimpleFinHolding {
            symbol: "ZERO".into(),
            shares: 0.0,
            market_value: Some(50.0),
            cost_basis: Some(40.0),
            currency: None,
        };
        assert_eq!(holding_unit_prices(&zero), (None, None));
    }

    fn brokerage_account() -> SimpleFinAccount {
        SimpleFinAccount {
            id: "act-1".into(),
            name: "Self-Directed Brokerage".into(),
            currency: "USD".into(),
            balance: Some(1000.0),
            balance_date: None,
            institution: Some("Wealthsimple".into()),
            holdings: vec![SimpleFinHolding {
                symbol: "AAPL".into(),
                shares: 10.0,
                market_value: Some(1500.0),
                cost_basis: Some(1000.0),
                currency: Some("USD".into()),
            }],
        }
    }

    #[test]
    fn reconcile_inserts_then_updates_by_connector_ref() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        let account = brokerage_account();
        let id = upsert_account(&conn, &account, "2025-01-01", "2025-01-01T00:00:00Z").unwrap();
        let holdings = replace_holdings(&conn, id, &account, "2025-01-01T00:00:00Z").unwrap();
        assert_eq!(holdings, 1);

        // The account is created with the brokerage type and SimpleFIN connector metadata.
        let (kind, account_type): (String, String) = conn
            .query_row(
                "SELECT connector_kind, account_type FROM accounts WHERE connector_ref = 'act-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "simplefin");
        assert_eq!(account_type, "brokerage");

        // Balance snapshot + derived per-share holding figures land in the schema.
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 1000.0);
        let (qty, last_price, avg_cost): (f64, f64, f64) = conn
            .query_row(
                "SELECT quantity, last_price, average_cost FROM holdings WHERE account_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((qty, last_price, avg_cost), (10.0, 150.0, 100.0));

        // A second sync updates the same row (no duplicate) and replaces holdings.
        let mut updated = brokerage_account();
        updated.balance = Some(1200.0);
        updated.holdings.clear();
        let id2 = upsert_account(&conn, &updated, "2025-01-01", "2025-01-02T00:00:00Z").unwrap();
        let holdings2 = replace_holdings(&conn, id2, &updated, "2025-01-02T00:00:00Z").unwrap();
        assert_eq!(id2, id);
        assert_eq!(holdings2, 0);

        let account_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(account_rows, 1);
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 1200.0);
        let holding_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM holdings WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(holding_rows, 0);
    }

    #[test]
    fn reconcile_preserves_a_user_corrected_currency() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        // First sync: SimpleFIN mislabels this Jamaican account as CAD.
        let mut account = brokerage_account();
        account.id = "act-jm".into();
        account.currency = "CAD".into();
        account.balance = Some(75000.0);
        account.holdings.clear();
        let id = upsert_account(&conn, &account, "2025-01-01", "2025-01-01T00:00:00Z").unwrap();

        // The user corrects it to JMD (as `update_account_currency` would).
        conn.execute(
            "UPDATE accounts SET currency = 'JMD' WHERE id = ?1",
            params![id],
        )
        .unwrap();

        // A later sync still reports CAD, but the correction must stick and the new snapshot
        // must inherit the stored JMD currency.
        let id2 = upsert_account(&conn, &account, "2025-01-02", "2025-01-02T00:00:00Z").unwrap();
        assert_eq!(id2, id);

        let currency: String = conn
            .query_row("SELECT currency FROM accounts WHERE id = ?1", params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(currency, "JMD");

        let snapshot_currency: String = conn
            .query_row(
                "SELECT currency FROM balance_snapshots WHERE account_id = ?1 AND snapshot_date = '2025-01-02'",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_currency, "JMD");
    }

    #[test]
    fn settings_roundtrip_and_delete() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        assert_eq!(get_setting(&conn, SETTING_LAST_SYNCED).unwrap(), None);
        set_setting(&conn, SETTING_LAST_SYNCED, "2025-01-01T00:00:00Z").unwrap();
        assert_eq!(
            get_setting(&conn, SETTING_LAST_SYNCED).unwrap().as_deref(),
            Some("2025-01-01T00:00:00Z")
        );
        delete_setting(&conn, SETTING_LAST_SYNCED).unwrap();
        assert_eq!(get_setting(&conn, SETTING_LAST_SYNCED).unwrap(), None);
    }
}
