use std::collections::BTreeSet;

use serde::Serialize;
use tauri::State;

use crate::db::AppDb;
use crate::fx as fx_module;

#[derive(Debug, Serialize)]
pub struct FxRateRow {
    pub id: i64,
    pub from_currency: String,
    pub to_currency: String,
    pub rate: f64,
    pub rate_date: String,
    pub source: String,
    pub created_at: String,
}

/// Return all stored FX rates, newest first.
#[tauri::command]
pub fn get_fx_rates(db: State<AppDb>) -> Result<Vec<FxRateRow>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, from_currency, to_currency, rate, rate_date, source, created_at \
             FROM fx_rates ORDER BY rate_date DESC, id DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| {
            Ok(FxRateRow {
                id: r.get(0)?,
                from_currency: r.get(1)?,
                to_currency: r.get(2)?,
                rate: r.get(3)?,
                rate_date: r.get(4)?,
                source: r.get(5)?,
                created_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

/// Fetch fresh USD→X rates from Yahoo Finance for every currency the user's active accounts
/// use (plus CAD, so the CAD total always works), and persist them. USD is the pivot, so a
/// single rate per currency lets net worth convert any balance into both USD and CAD.
///
/// Resilient: if one currency fails (e.g. an unknown symbol) the rest still update. An error is
/// only returned if *nothing* could be fetched.
#[tauri::command]
pub async fn refresh_fx_rates(db: State<'_, AppDb>) -> Result<Vec<FxRateRow>, String> {
    // Collect the currencies to refresh up front, then release the lock before any network I/O.
    let currencies: Vec<String> = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut set: BTreeSet<String> = BTreeSet::new();
        set.insert("CAD".to_string());
        let mut stmt = conn
            .prepare("SELECT DISTINCT currency FROM accounts WHERE is_active = 1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            set.insert(row.map_err(|e| e.to_string())?);
        }
        // USD is the pivot (always 1.0) — never fetched.
        set.into_iter().filter(|c| c != "USD").collect()
    };

    let client = reqwest::Client::new();
    let mut fetched = 0usize;
    let mut last_err: Option<String> = None;
    for currency in &currencies {
        match fx_module::fetch_usd_rate(&client, currency).await {
            Ok((rate, rate_date)) => {
                let conn = db.0.lock().map_err(|e| e.to_string())?;
                fx_module::store_usd_rate(&conn, currency, rate, &rate_date)
                    .map_err(|e| e.to_string())?;
                fetched += 1;
            }
            Err(e) => last_err = Some(format!("{currency}: {e}")),
        }
    }

    if fetched == 0 {
        if let Some(err) = last_err {
            return Err(err);
        }
    }

    get_fx_rates(db)
}
