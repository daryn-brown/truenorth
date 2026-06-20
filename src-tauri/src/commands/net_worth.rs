use serde::Serialize;
use std::collections::HashMap;
use rusqlite::Connection;
use tauri::State;

use crate::db::AppDb;
use crate::fx::load_latest_rates;

#[derive(Debug, Serialize)]
pub struct AccountNetWorth {
    pub account_id: i64,
    pub account_name: String,
    pub institution: String,
    pub account_type: String,
    pub jurisdiction: String,
    pub balance: f64,
    pub currency: String,
    pub balance_usd: f64,
    pub balance_cad: f64,
    pub snapshot_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NetWorthResponse {
    pub total_usd: f64,
    pub total_cad: f64,
    pub accounts: Vec<AccountNetWorth>,
    pub usd_cad_rate: Option<f64>,
    pub cad_usd_rate: Option<f64>,
    pub rate_date: Option<String>,
}

/// Compute the current net worth across all active accounts.
///
/// Uses the most recent FX rate in the database. If no rate is available,
/// values in the non-native currency are returned as 0.
#[tauri::command]
pub fn get_net_worth(db: State<AppDb>) -> Result<NetWorthResponse, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let rates = load_latest_rates(&conn).map_err(|e| e.to_string())?;
    let (usd_cad, cad_usd, rate_date) = match rates {
        Some((u, c, d)) => (Some(u), Some(c), Some(d)),
        None => (None, None, None),
    };

    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                a.id, a.name, a.institution, a.account_type, a.jurisdiction,
                a.currency,
                bs.balance       AS balance,
                bs.snapshot_date AS snapshot_date
            FROM accounts a
            LEFT JOIN balance_snapshots bs ON bs.id = (
                SELECT id FROM balance_snapshots
                WHERE account_id = a.id
                ORDER BY snapshot_date DESC
                LIMIT 1
            )
            WHERE a.is_active = 1
            "#,
        )
        .map_err(|e| e.to_string())?;

    let account_rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<f64>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut total_usd = 0.0_f64;
    let mut total_cad = 0.0_f64;
    let mut accounts = Vec::with_capacity(account_rows.len());

    for (id, name, institution, account_type, jurisdiction, currency, balance_opt, snapshot_date) in
        account_rows
    {
        let balance = balance_opt.unwrap_or(0.0);

        let (balance_usd, balance_cad) = convert_balance(balance, &currency, usd_cad, cad_usd);
        total_usd += balance_usd;
        total_cad += balance_cad;

        accounts.push(AccountNetWorth {
            account_id: id,
            account_name: name,
            institution,
            account_type,
            jurisdiction,
            balance,
            currency,
            balance_usd,
            balance_cad,
            snapshot_date,
        });
    }

    Ok(NetWorthResponse {
        total_usd,
        total_cad,
        accounts,
        usd_cad_rate: usd_cad,
        cad_usd_rate: cad_usd,
        rate_date,
    })
}

/// Convert a balance in `currency` to both USD and CAD.
fn convert_balance(
    balance: f64,
    currency: &str,
    usd_cad: Option<f64>,
    cad_usd: Option<f64>,
) -> (f64, f64) {
    match currency {
        "USD" => {
            let cad = usd_cad.map(|r| balance * r).unwrap_or(0.0);
            (balance, cad)
        }
        "CAD" => {
            let usd = cad_usd.map(|r| balance * r).unwrap_or(0.0);
            (usd, balance)
        }
        _ => (0.0, 0.0),
    }
}

// ---------------------------------------------------------------------------
// Net-worth history (time series)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq)]
pub struct NetWorthHistoryPoint {
    /// ISO date YYYY-MM-DD.
    pub date: String,
    pub total_usd: f64,
    pub total_cad: f64,
}

/// Total net worth over time across active accounts.
///
/// For each date on which any account has a balance snapshot, every account's most
/// recent balance *as of that date* is carried forward (accounts contribute 0 before
/// their first snapshot), converted with the latest stored FX rate, and summed. Using
/// a single consistent FX rate keeps the trend driven by balances, not FX noise.
#[tauri::command]
pub fn get_net_worth_history(db: State<AppDb>) -> Result<Vec<NetWorthHistoryPoint>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    compute_net_worth_history(&conn).map_err(|e| e.to_string())
}

fn compute_net_worth_history(conn: &Connection) -> rusqlite::Result<Vec<NetWorthHistoryPoint>> {
    let (usd_cad, cad_usd) = match load_latest_rates(conn)? {
        Some((u, c, _)) => (Some(u), Some(c)),
        None => (None, None),
    };

    // account_id -> currency (active accounts only)
    let mut currency_of: HashMap<i64, String> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, currency FROM accounts WHERE is_active = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, currency) = row?;
            currency_of.insert(id, currency);
        }
    }

    // All snapshots for active accounts, oldest first.
    let snapshots: Vec<(i64, String, f64)> = {
        let mut stmt = conn.prepare(
            "SELECT bs.account_id, bs.snapshot_date, bs.balance \
             FROM balance_snapshots bs \
             JOIN accounts a ON a.id = bs.account_id \
             WHERE a.is_active = 1 \
             ORDER BY bs.snapshot_date ASC, bs.account_id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    // Walk dates in order, carrying each account's latest balance forward.
    let mut current: HashMap<i64, f64> = HashMap::new();
    let mut points: Vec<NetWorthHistoryPoint> = Vec::new();
    let mut idx = 0;
    while idx < snapshots.len() {
        let date = snapshots[idx].1.clone();
        while idx < snapshots.len() && snapshots[idx].1 == date {
            current.insert(snapshots[idx].0, snapshots[idx].2);
            idx += 1;
        }

        let mut total_usd = 0.0_f64;
        let mut total_cad = 0.0_f64;
        for (account_id, balance) in &current {
            let currency = currency_of
                .get(account_id)
                .map(|s| s.as_str())
                .unwrap_or("USD");
            let (usd, cad) = convert_balance(*balance, currency, usd_cad, cad_usd);
            total_usd += usd;
            total_cad += cad;
        }

        points.push(NetWorthHistoryPoint {
            date,
            total_usd,
            total_cad,
        });
    }

    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::apply_schema;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        conn
    }

    fn add_account(conn: &Connection, name: &str, currency: &str) -> i64 {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES (?1, 'Inst', 'savings', ?2, 'CA')",
            rusqlite::params![name, currency],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_snapshot(conn: &Connection, account_id: i64, date: &str, balance: f64, currency: &str) {
        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots (account_id, snapshot_date, balance, currency) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![account_id, date, balance, currency],
        )
        .unwrap();
    }

    #[test]
    fn history_carries_balances_forward() {
        let conn = setup();
        // USD -> CAD = 2.0 so conversions are easy to assert.
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2025-01-01')",
            [],
        )
        .unwrap();

        let usd = add_account(&conn, "US Checking", "USD");
        let cad = add_account(&conn, "CA Savings", "CAD");

        add_snapshot(&conn, usd, "2025-01-01", 100.0, "USD");
        add_snapshot(&conn, cad, "2025-02-01", 50.0, "CAD");
        add_snapshot(&conn, usd, "2025-03-01", 200.0, "USD");

        let series = compute_net_worth_history(&conn).unwrap();
        assert_eq!(series.len(), 3);

        // 2025-01-01: only USD 100 -> 100 USD / 200 CAD
        assert_eq!(series[0].date, "2025-01-01");
        assert_eq!(series[0].total_usd, 100.0);
        assert_eq!(series[0].total_cad, 200.0);

        // 2025-02-01: USD 100 (carried) + CAD 50 -> 125 USD / 250 CAD
        assert_eq!(series[1].total_cad, 250.0);
        assert_eq!(series[1].total_usd, 125.0);

        // 2025-03-01: USD 200 (updated) + CAD 50 -> 225 USD / 450 CAD
        assert_eq!(series[2].total_usd, 225.0);
        assert_eq!(series[2].total_cad, 450.0);
    }

    #[test]
    fn history_is_empty_without_snapshots() {
        let conn = setup();
        add_account(&conn, "Empty", "CAD");
        assert!(compute_net_worth_history(&conn).unwrap().is_empty());
    }
}
