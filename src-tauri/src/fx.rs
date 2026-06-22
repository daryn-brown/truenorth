use chrono::Utc;
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::collections::HashMap;
use thiserror::Error;

const YAHOO_QUOTE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";

#[derive(Debug, Error)]
pub enum FxError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Unexpected Yahoo Finance response: {0}")]
    ParseError(String),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, Deserialize)]
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Debug, Deserialize)]
struct YahooChart {
    result: Option<Vec<YahooResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct YahooResult {
    meta: YahooMeta,
}

#[derive(Debug, Deserialize)]
struct YahooMeta {
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "previousClose")]
    previous_close: Option<f64>,
}

/// Fetch the current USD→`currency` rate from Yahoo Finance (e.g. `USDJMD=X`).
/// Returns how many units of `currency` equal 1 USD, plus the ISO date it was fetched.
/// `USD` short-circuits to 1.0 (it is the pivot currency).
pub async fn fetch_usd_rate(client: &Client, currency: &str) -> Result<(f64, String), FxError> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    if currency.eq_ignore_ascii_case("USD") {
        return Ok((1.0, date));
    }

    let url = format!("{YAHOO_QUOTE_URL}/USD{currency}=X?interval=1d&range=1d");
    let resp: YahooChartResponse = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.chart.error {
        return Err(FxError::ParseError(format!("Yahoo error: {err}")));
    }

    let rate = resp
        .chart
        .result
        .as_deref()
        .and_then(|r| r.first())
        .and_then(|r| {
            r.meta
                .regular_market_price
                .or(r.meta.previous_close)
        })
        .ok_or_else(|| FxError::ParseError("No price in Yahoo response".into()))?;

    Ok((rate, date))
}

/// Upsert a USD→`currency` rate (and its CUR→USD inverse) into the fx_rates table.
/// `usd_to_cur` is how many units of `currency` equal 1 USD.
pub fn store_usd_rate(
    conn: &Connection,
    currency: &str,
    usd_to_cur: f64,
    rate_date: &str,
) -> Result<(), rusqlite::Error> {
    if currency.eq_ignore_ascii_case("USD") {
        return Ok(());
    }

    conn.execute(
        "INSERT OR REPLACE INTO fx_rates (from_currency, to_currency, rate, rate_date, source) \
         VALUES ('USD', ?1, ?2, ?3, 'yahoo')",
        params![currency, usd_to_cur, rate_date],
    )?;
    if usd_to_cur != 0.0 {
        conn.execute(
            "INSERT OR REPLACE INTO fx_rates (from_currency, to_currency, rate, rate_date, source) \
             VALUES (?1, 'USD', ?2, ?3, 'yahoo')",
            params![currency, 1.0 / usd_to_cur, rate_date],
        )?;
    }
    Ok(())
}

/// Load the latest USD→CAD and CAD→USD rates from the database.
/// Returns `(usd_cad_rate, cad_usd_rate, rate_date)` or None if no rates stored.
pub fn load_latest_rates(
    conn: &Connection,
) -> Result<Option<(f64, f64, String)>, rusqlite::Error> {
    let usd_cad: Option<(f64, String)> = conn
        .query_row(
            "SELECT rate, rate_date FROM fx_rates \
             WHERE from_currency = 'USD' AND to_currency = 'CAD' \
             ORDER BY rate_date DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    match usd_cad {
        None => Ok(None),
        Some((rate, date)) => {
            let cad_usd = 1.0 / rate;
            Ok(Some((rate, cad_usd, date)))
        }
    }
}

/// Load the latest USD→X rate for every currency we have stored, as a map of
/// `currency -> units of that currency per 1 USD`. `USD` is always present as `1.0`.
/// This is the canonical table used to convert any balance into USD (and onward to CAD).
pub fn load_usd_rates(conn: &Connection) -> Result<HashMap<String, f64>, rusqlite::Error> {
    let mut rates = HashMap::new();
    rates.insert("USD".to_string(), 1.0);

    let mut stmt = conn.prepare(
        "SELECT to_currency, rate FROM fx_rates f \
         WHERE from_currency = 'USD' \
           AND rate_date = ( \
               SELECT MAX(rate_date) FROM fx_rates \
               WHERE from_currency = 'USD' AND to_currency = f.to_currency \
           )",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    for row in rows {
        let (currency, rate) = row?;
        rates.insert(currency, rate);
    }
    Ok(rates)
}

trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
