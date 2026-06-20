use chrono::Utc;
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::Deserialize;
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

/// Fetch the current USD→CAD rate from Yahoo Finance.
/// Returns the rate and the ISO date it was fetched.
pub async fn fetch_usd_cad(client: &Client) -> Result<(f64, String), FxError> {
    let url = format!("{YAHOO_QUOTE_URL}/USDCAD=X?interval=1d&range=1d");
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

    let date = Utc::now().format("%Y-%m-%d").to_string();
    Ok((rate, date))
}

/// Upsert both USD→CAD and CAD→USD into the fx_rates table.
pub fn store_fx_rate(
    conn: &Connection,
    usd_cad: f64,
    rate_date: &str,
) -> Result<(), rusqlite::Error> {
    let cad_usd = 1.0 / usd_cad;

    conn.execute(
        "INSERT OR REPLACE INTO fx_rates (from_currency, to_currency, rate, rate_date, source) \
         VALUES ('USD', 'CAD', ?1, ?2, 'yahoo')",
        params![usd_cad, rate_date],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO fx_rates (from_currency, to_currency, rate, rate_date, source) \
         VALUES ('CAD', 'USD', ?1, ?2, 'yahoo')",
        params![cad_usd, rate_date],
    )?;
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
