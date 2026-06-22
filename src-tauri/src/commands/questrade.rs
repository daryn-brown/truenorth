//! Questrade Tauri commands: connect with a refresh token, sync balances + holdings, disconnect.
//!
//! Unlike SimpleFIN (banks via MX) and SnapTrade (aggregated brokerages), this talks **directly**
//! to Questrade's free personal REST API. The user pastes a manual-authorization refresh token,
//! which we exchange for a short-lived access token + data host, then read accounts, balances, and
//! positions. We persist the **rotated** refresh token immediately (it is single-use), then write
//! one balance snapshot per account using `totalEquity` (cash **and** equity) so net worth reflects
//! the true account value — the gap SimpleFIN leaves for Questrade.
//!
//! "Complement, not overwrite": Questrade owns its own account rows (`connector_kind = 'questrade'`)
//! and never mutates SimpleFIN/manual rows. On each sync it deactivates any *aggregator-managed*
//! (SimpleFIN/SnapTrade) account that points at Questrade, so the redundant cash-only duplicate
//! stops double-counting net worth. As elsewhere, the SQLite mutex is never held across an `.await`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::connector::questrade::{
    refresh_access_token, QuestradeAccount, QuestradeBalance, QuestradeClient, QuestradeError,
    QuestradePosition,
};
use crate::db::secrets::{self, QUESTRADE_REFRESH_TOKEN};
use crate::db::AppDb;

const SETTING_LAST_SYNCED: &str = "questrade_last_synced_at";

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct QuestradeStatus {
    /// A refresh token is stored (the user has connected their Questrade API app).
    pub is_connected: bool,
    pub last_synced_at: Option<String>,
    /// Number of active accounts connected directly via Questrade.
    pub account_count: i64,
}

#[derive(Debug, Serialize)]
pub struct QuestradeSyncSummary {
    pub accounts_synced: usize,
    pub holdings_synced: usize,
    /// Redundant cash-only duplicates (the same Questrade account aggregated via SimpleFIN/SnapTrade)
    /// that were deactivated so net worth isn't double-counted.
    pub duplicates_hidden: usize,
    pub synced_at: String,
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

/// Map a Questrade account type ("TFSA", "RRSP", "Margin", "Cash", "FHSA", …) to one of
/// TrueNorth's account-type ids. Registered/locked-in plans collapse to `rrsp`; ordinary Cash or
/// Margin accounts (and anything unrecognised) are plain `brokerage`.
fn map_account_type(raw: &str) -> String {
    let hay = raw.to_uppercase();
    let has = |needle: &str| hay.contains(needle);

    let kind = if has("TFSA") {
        "tfsa"
    } else if has("FHSA") {
        "fhsa"
    } else if has("RRSP") || has("RSP") || has("RRIF") || has("RIF") || has("LIRA") || has("LIF") {
        // Registered retirement and locked-in plans all map to the rrsp bucket.
        "rrsp"
    } else if has("RESP") {
        // No dedicated RESP type; keep it visible under "other".
        "other"
    } else {
        // Cash, Margin, and unknown types are ordinary brokerage accounts.
        "brokerage"
    };
    kind.to_string()
}

/// Build a stable, human-readable account name. Questrade gives a type and a number but no
/// nickname, so we combine the type with the last four digits to disambiguate multiple accounts of
/// the same type (e.g. two TFSAs).
fn account_display_name(raw_type: &str, number: &str) -> String {
    let label = match raw_type.trim() {
        "" => "Account",
        other => other,
    };
    let trimmed = number.trim();
    let last4: String = if trimmed.chars().count() > 4 {
        trimmed.chars().skip(trimmed.chars().count() - 4).collect()
    } else {
        trimmed.to_string()
    };
    if last4.is_empty() {
        label.to_string()
    } else {
        format!("{label} ••{last4}")
    }
}

/// Pick the balance row to drive net worth: prefer the account's home currency (CAD for Questrade),
/// then USD, then whatever is first.
fn pick_balance(balances: &[QuestradeBalance]) -> Option<&QuestradeBalance> {
    balances
        .iter()
        .find(|b| b.currency.eq_ignore_ascii_case("CAD"))
        .or_else(|| {
            balances
                .iter()
                .find(|b| b.currency.eq_ignore_ascii_case("USD"))
        })
        .or_else(|| balances.first())
}

/// Total account value for a balance row: Questrade's `totalEquity` (cash + market value), falling
/// back to summing whichever of cash/market value is present.
fn balance_total(b: &QuestradeBalance) -> Option<f64> {
    b.total_equity.or_else(|| match (b.cash, b.market_value) {
        (None, None) => None,
        (cash, market) => Some(cash.unwrap_or(0.0) + market.unwrap_or(0.0)),
    })
}

/// Only sync accounts that are open. Questrade may also return closed accounts; skip those so we
/// don't resurrect them. A missing status is treated as open.
fn account_is_open(a: &QuestradeAccount) -> bool {
    a.status
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("Active"))
        .unwrap_or(true)
}

/// Turn a Questrade error into a user-facing message.
fn friendly(e: QuestradeError) -> String {
    if e.is_auth() {
        "Questrade rejected the connection. Re-authorize in your Questrade API Centre — the refresh \
         token may have expired (after ~7 days unused) or already been used — then paste a fresh \
         token."
            .into()
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// DB reconcile helpers (synchronous — never run while awaiting)
// ---------------------------------------------------------------------------

/// Upsert one Questrade account (keyed by `connector_ref` = account number), write today's balance
/// snapshot from `totalEquity`, and replace its holdings. Returns the number of holdings written.
fn reconcile_account(
    conn: &Connection,
    account: &QuestradeAccount,
    balances: &[QuestradeBalance],
    positions: &[QuestradePosition],
    today: &str,
    now: &str,
) -> rusqlite::Result<usize> {
    let picked = pick_balance(balances);
    let currency = picked
        .map(|b| b.currency.clone())
        .unwrap_or_else(|| "CAD".to_string());
    let account_type = map_account_type(&account.account_type);
    let name = account_display_name(&account.account_type, &account.number);

    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM accounts WHERE connector_kind = 'questrade' AND connector_ref = ?1",
            params![account.number],
            |r| r.get(0),
        )
        .optional()?;

    // Questrade accounts are always Canadian, regardless of the currency a balance is expressed in.
    let account_id = if let Some(id) = existing_id {
        conn.execute(
            "UPDATE accounts SET name = ?1, institution = 'Questrade', account_type = ?2, \
             currency = ?3, jurisdiction = 'CA', is_active = 1, updated_at = ?4 WHERE id = ?5",
            params![name, account_type, currency, now, id],
        )?;
        id
    } else {
        conn.execute(
            "INSERT INTO accounts \
             (name, institution, account_type, currency, jurisdiction, connector_kind, connector_ref) \
             VALUES (?1, 'Questrade', ?2, ?3, 'CA', 'questrade', ?4)",
            params![name, account_type, currency, account.number],
        )?;
        conn.last_insert_rowid()
    };

    if let Some(total) = picked.and_then(balance_total) {
        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, ?2, ?3, ?4, 'questrade')",
            params![account_id, today, total, currency],
        )?;
    }

    conn.execute(
        "DELETE FROM holdings WHERE account_id = ?1",
        params![account_id],
    )?;
    let mut count = 0usize;
    for p in positions {
        // Questrade positions carry no currency, so they inherit the account currency.
        let last_price_at = p.current_price.map(|_| now.to_string());
        conn.execute(
            "INSERT OR REPLACE INTO holdings \
             (account_id, symbol, quantity, average_cost, currency, last_price, last_price_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                p.symbol,
                p.open_quantity,
                p.average_entry_price,
                currency,
                p.current_price,
                last_price_at,
                now
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Deactivate any active aggregator-managed (SimpleFIN/SnapTrade) account that points at Questrade.
/// These are the redundant, often cash-only duplicates of the accounts we now sync directly; soft-
/// deleting them (history preserved) stops net worth from double-counting. Manual accounts are
/// deliberately left alone. Returns the number hidden.
fn deactivate_duplicate_aggregator_accounts(
    conn: &Connection,
    now: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE accounts SET is_active = 0, updated_at = ?1 \
         WHERE is_active = 1 AND connector_kind IN ('simplefin', 'snaptrade') \
         AND lower(institution) LIKE '%questrade%'",
        params![now],
    )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Report whether Questrade is connected, plus last-synced time and connected-account count.
#[tauri::command]
pub fn questrade_get_status(db: State<AppDb>) -> Result<QuestradeStatus, String> {
    let (last_synced_at, account_count) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let last_synced_at = get_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        let account_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'questrade' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        (last_synced_at, account_count)
    };

    let refresh_token = secrets::get_secret(QUESTRADE_REFRESH_TOKEN).map_err(|e| e.to_string())?;

    Ok(QuestradeStatus {
        is_connected: refresh_token.is_some(),
        last_synced_at,
        account_count,
    })
}

/// Connect Questrade: exchange the pasted refresh token (validating it lists accounts), then store
/// the **rotated** refresh token in the keychain. The token the user pasted is now spent.
#[tauri::command]
pub async fn questrade_connect(
    db: State<'_, AppDb>,
    refresh_token: String,
) -> Result<QuestradeStatus, String> {
    let refresh_token = refresh_token.trim().to_string();
    if refresh_token.is_empty() {
        return Err("Paste the refresh token from your Questrade API Centre first.".into());
    }

    // Exchange the one-time token for an access token + data host, then confirm it actually works.
    let tokens = refresh_access_token(&refresh_token)
        .await
        .map_err(friendly)?;
    QuestradeClient::new(tokens.api_server.clone(), tokens.access_token.clone())
        .check()
        .await
        .map_err(friendly)?;

    // Persist the rotated refresh token — the pasted one is now invalid.
    secrets::set_secret(QUESTRADE_REFRESH_TOKEN, &tokens.refresh_token)
        .map_err(|e| e.to_string())?;

    questrade_get_status(db)
}

/// Pull accounts + balances + positions from Questrade and reconcile them into the local DB.
/// Writes one `totalEquity` balance snapshot per account (so net worth updates automatically) and
/// hides any aggregator duplicate that points at Questrade.
#[tauri::command]
pub async fn questrade_sync(db: State<'_, AppDb>) -> Result<QuestradeSyncSummary, String> {
    let refresh_token = secrets::get_secret(QUESTRADE_REFRESH_TOKEN)
        .map_err(|e| e.to_string())?
        .ok_or("Connect Questrade before syncing.")?;

    // Exchange the refresh token (this rotates it) and persist the new one immediately — before any
    // data calls — so a later failure can never strand us with a spent token.
    let tokens = refresh_access_token(&refresh_token)
        .await
        .map_err(friendly)?;
    secrets::set_secret(QUESTRADE_REFRESH_TOKEN, &tokens.refresh_token)
        .map_err(|e| e.to_string())?;

    // Fetch everything over the network first — no DB lock is held across an await.
    let client = QuestradeClient::new(tokens.api_server, tokens.access_token);
    let accounts = client.list_accounts().await.map_err(friendly)?;

    let mut fetched: Vec<(
        QuestradeAccount,
        Vec<QuestradeBalance>,
        Vec<QuestradePosition>,
    )> = Vec::new();
    for account in accounts {
        if !account_is_open(&account) {
            continue;
        }
        let balances = client
            .account_balances(&account.number)
            .await
            .map_err(friendly)?;
        let positions = client
            .account_positions(&account.number)
            .await
            .map_err(friendly)?;
        fetched.push((account, balances, positions));
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut accounts_synced = 0usize;
    let mut holdings_synced = 0usize;
    let duplicates_hidden;

    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        for (account, balances, positions) in &fetched {
            holdings_synced += reconcile_account(&tx, account, balances, positions, &today, &now)
                .map_err(|e| e.to_string())?;
            accounts_synced += 1;
        }

        duplicates_hidden =
            deactivate_duplicate_aggregator_accounts(&tx, &now).map_err(|e| e.to_string())?;

        set_setting(&tx, SETTING_LAST_SYNCED, &now).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(QuestradeSyncSummary {
        accounts_synced,
        holdings_synced,
        duplicates_hidden,
        synced_at: now,
    })
}

/// Disconnect Questrade: remove the stored refresh token, clear the last-synced marker, and
/// deactivate the connected accounts. Historical snapshots are left untouched.
#[tauri::command]
pub fn questrade_disconnect(db: State<AppDb>) -> Result<QuestradeStatus, String> {
    secrets::delete_secret(QUESTRADE_REFRESH_TOKEN).map_err(|e| e.to_string())?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        delete_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE connector_kind = 'questrade'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    questrade_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_questrade_account_types() {
        assert_eq!(map_account_type("TFSA"), "tfsa");
        assert_eq!(map_account_type("FHSA"), "fhsa");
        assert_eq!(map_account_type("RRSP"), "rrsp");
        assert_eq!(map_account_type("Spousal RRSP"), "rrsp");
        assert_eq!(map_account_type("LIRA"), "rrsp");
        assert_eq!(map_account_type("RESP"), "other");
        assert_eq!(map_account_type("Margin"), "brokerage");
        assert_eq!(map_account_type("Cash"), "brokerage");
        assert_eq!(map_account_type(""), "brokerage");
    }

    #[test]
    fn display_name_combines_type_and_last_four() {
        assert_eq!(account_display_name("TFSA", "26598145"), "TFSA ••8145");
        assert_eq!(account_display_name("", "12"), "Account ••12");
        assert_eq!(account_display_name("Margin", ""), "Margin");
    }

    #[test]
    fn pick_balance_prefers_cad_then_usd() {
        let usd = QuestradeBalance {
            currency: "USD".into(),
            cash: Some(100.0),
            market_value: Some(900.0),
            total_equity: Some(1000.0),
        };
        let cad = QuestradeBalance {
            currency: "CAD".into(),
            cash: Some(1519.56),
            market_value: Some(24480.44),
            total_equity: Some(26000.0),
        };
        assert_eq!(
            pick_balance(&[usd.clone(), cad.clone()]).unwrap().currency,
            "CAD"
        );
        assert_eq!(
            pick_balance(std::slice::from_ref(&usd)).unwrap().currency,
            "USD"
        );
        assert!(pick_balance(&[]).is_none());
    }

    #[test]
    fn balance_total_prefers_total_equity_then_sums() {
        assert_eq!(
            balance_total(&QuestradeBalance {
                currency: "CAD".into(),
                cash: Some(1.0),
                market_value: Some(2.0),
                total_equity: Some(26000.0),
            }),
            Some(26000.0)
        );
        assert_eq!(
            balance_total(&QuestradeBalance {
                currency: "CAD".into(),
                cash: Some(1519.56),
                market_value: Some(24480.44),
                total_equity: None,
            }),
            Some(26000.0)
        );
        assert_eq!(
            balance_total(&QuestradeBalance {
                currency: "CAD".into(),
                cash: None,
                market_value: None,
                total_equity: None,
            }),
            None
        );
    }

    #[test]
    fn account_open_status_filter() {
        let active = QuestradeAccount {
            number: "1".into(),
            account_type: "TFSA".into(),
            status: Some("Active".into()),
        };
        let closed = QuestradeAccount {
            number: "2".into(),
            account_type: "TFSA".into(),
            status: Some("Closed".into()),
        };
        let unknown = QuestradeAccount {
            number: "3".into(),
            account_type: "TFSA".into(),
            status: None,
        };
        assert!(account_is_open(&active));
        assert!(!account_is_open(&closed));
        assert!(account_is_open(&unknown));
    }

    fn tfsa_account() -> QuestradeAccount {
        QuestradeAccount {
            number: "26598145".into(),
            account_type: "TFSA".into(),
            status: Some("Active".into()),
        }
    }

    fn cad_balance(total: f64) -> QuestradeBalance {
        QuestradeBalance {
            currency: "CAD".into(),
            cash: Some(1519.56),
            market_value: Some(total - 1519.56),
            total_equity: Some(total),
        }
    }

    #[test]
    fn reconcile_writes_total_equity_snapshot_and_holdings() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        let account = tfsa_account();
        let balances = vec![cad_balance(26000.0)];
        let positions = vec![QuestradePosition {
            symbol: "VFV.TO".into(),
            open_quantity: 100.0,
            current_price: Some(120.0),
            average_entry_price: Some(95.5),
            current_market_value: Some(12000.0),
        }];

        let holdings = reconcile_account(
            &conn,
            &account,
            &balances,
            &positions,
            "2025-01-01",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(holdings, 1);

        // The account is created with the questrade connector metadata and tfsa type.
        let (kind, account_type, jurisdiction, currency): (String, String, String, String) = conn
            .query_row(
                "SELECT connector_kind, account_type, jurisdiction, currency FROM accounts \
                 WHERE connector_ref = '26598145'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(kind, "questrade");
        assert_eq!(account_type, "tfsa");
        assert_eq!(jurisdiction, "CA");
        assert_eq!(currency, "CAD");

        // The snapshot is the full account value (cash + equity), not just cash.
        let id: i64 = conn
            .query_row(
                "SELECT id FROM accounts WHERE connector_ref = '26598145'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 26000.0);

        let (qty, last_price, avg_cost, hcur): (f64, f64, f64, String) = conn
            .query_row(
                "SELECT quantity, last_price, average_cost, currency FROM holdings \
                 WHERE account_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((qty, last_price, avg_cost), (100.0, 120.0, 95.5));
        assert_eq!(hcur, "CAD");

        // A second sync updates the same row (no duplicate) and replaces holdings.
        let id2 = {
            reconcile_account(
                &conn,
                &account,
                &[cad_balance(27000.0)],
                &[],
                "2025-01-02",
                "2025-01-02T00:00:00Z",
            )
            .unwrap();
            conn.query_row(
                "SELECT id FROM accounts WHERE connector_ref = '26598145'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(id2, id);
        let account_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'questrade'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(account_rows, 1);
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
    fn dedup_hides_only_aggregator_questrade_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        // A SimpleFIN-aggregated Questrade account (the redundant cash-only duplicate).
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
             connector_kind, connector_ref, is_active) \
             VALUES ('TFSA', 'Questrade', 'tfsa', 'CAD', 'CA', 'simplefin', 'sf-1', 1)",
            [],
        )
        .unwrap();
        // A SimpleFIN account at a different institution — must stay active.
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
             connector_kind, connector_ref, is_active) \
             VALUES ('Chequing', 'Chase', 'chequing', 'USD', 'US', 'simplefin', 'sf-2', 1)",
            [],
        )
        .unwrap();
        // A manual account that mentions Questrade — manual rows are never touched.
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
             connector_kind, is_active) \
             VALUES ('Questrade Equity', 'Questrade', 'brokerage', 'CAD', 'CA', 'manual', 1)",
            [],
        )
        .unwrap();

        let hidden =
            deactivate_duplicate_aggregator_accounts(&conn, "2025-01-01T00:00:00Z").unwrap();
        assert_eq!(hidden, 1);

        let active_simplefin_questrade: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'simplefin' \
                 AND lower(institution) LIKE '%questrade%' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active_simplefin_questrade, 0);

        let active_total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE is_active = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Chase + the manual Questrade account remain active.
        assert_eq!(active_total, 2);
    }
}
