use serde::Serialize;
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
