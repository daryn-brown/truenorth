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

/// Fetch fresh USD/CAD rates from Yahoo Finance and persist them.
#[tauri::command]
pub async fn refresh_fx_rates(db: State<'_, AppDb>) -> Result<Vec<FxRateRow>, String> {
    let client = reqwest::Client::new();
    let (usd_cad, rate_date) = fx_module::fetch_usd_cad(&client)
        .await
        .map_err(|e| e.to_string())?;

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        fx_module::store_fx_rate(&conn, usd_cad, &rate_date).map_err(|e| e.to_string())?;
    }

    get_fx_rates(db)
}
