use std::collections::BTreeSet;

use chrono::Utc;
use rusqlite::Connection;
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
    let (fetched, last_err) = fetch_and_store_all(&db).await?;
    if fetched == 0 {
        if let Some(err) = last_err {
            return Err(err);
        }
    }
    get_fx_rates(db)
}

/// Like [`refresh_fx_rates`], but only hits the network when the stored rates aren't already from
/// today — a cheap "daily" auto-refresh the dashboard can call on every load. A network failure is
/// swallowed so the dashboard still renders with whatever rates are already stored.
#[tauri::command]
pub async fn refresh_fx_rates_if_stale(db: State<'_, AppDb>) -> Result<Vec<FxRateRow>, String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let stale = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        rates_are_stale(&conn, &today).map_err(|e| e.to_string())?
    };
    if stale {
        let _ = fetch_and_store_all(&db).await;
    }
    get_fx_rates(db)
}

/// Fetch USD→X for every currency the active accounts use (plus CAD) and persist it. Returns how
/// many currencies succeeded and the last per-currency error, if any. Only a lock/DB failure is
/// fatal; individual fetch failures are tolerated so one bad symbol can't sink the batch.
async fn fetch_and_store_all(db: &State<'_, AppDb>) -> Result<(usize, Option<String>), String> {
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

    Ok((fetched, last_err))
}

/// Whether the newest stored USD rate predates `today` (YYYY-MM-DD). No rates at all counts as
/// stale so the first run fetches an initial set.
fn rates_are_stale(conn: &Connection, today: &str) -> rusqlite::Result<bool> {
    let latest: Option<String> = conn.query_row(
        "SELECT MAX(rate_date) FROM fx_rates WHERE from_currency = 'USD'",
        [],
        |r| r.get::<_, Option<String>>(0),
    )?;
    Ok(match latest {
        None => true,
        Some(date) => date.as_str() < today,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::apply_schema;
    use rusqlite::params;

    fn db_with_rate(date: Option<&str>) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        if let Some(d) = date {
            conn.execute(
                "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date, source) \
                 VALUES ('USD', 'CAD', 1.37, ?1, 'test')",
                params![d],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn stale_when_no_rates_stored() {
        let conn = db_with_rate(None);
        assert!(rates_are_stale(&conn, "2026-01-15").unwrap());
    }

    #[test]
    fn fresh_when_rate_is_from_today() {
        let conn = db_with_rate(Some("2026-01-15"));
        assert!(!rates_are_stale(&conn, "2026-01-15").unwrap());
    }

    #[test]
    fn stale_when_rate_predates_today() {
        let conn = db_with_rate(Some("2026-01-14"));
        assert!(rates_are_stale(&conn, "2026-01-15").unwrap());
    }
}
