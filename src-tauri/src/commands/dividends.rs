use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::commands::net_worth::{convert_balance, MoneyPair};
use crate::db::AppDb;
use crate::fx::load_usd_rates;

const YAHOO_CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const RESEARCH_SOURCE: &str = "Yahoo Finance";
const MAX_CONCURRENT_RESEARCH_REQUESTS: usize = 6;
const MAX_BROKER_PRICE_DISTANCE: f64 = 0.35;

type ResearchKey = (String, String);

#[derive(Debug, Clone)]
struct HoldingRow {
    account_id: i64,
    account_name: String,
    symbol: String,
    quantity: f64,
    currency: String,
    last_price: Option<f64>,
}

impl HoldingRow {
    fn key(&self) -> ResearchKey {
        (
            normalize_symbol(&self.symbol),
            self.currency.trim().to_uppercase(),
        )
    }
}

#[derive(Debug, Clone)]
struct ResearchRecord {
    symbol: String,
    holding_currency: String,
    lookup_symbol: String,
    quote_currency: String,
    annual_dividend_per_share: f64,
    current_price: Option<f64>,
    researched_on: String,
}

impl ResearchRecord {
    fn key(&self) -> ResearchKey {
        (self.symbol.clone(), self.holding_currency.clone())
    }
}

#[derive(Debug, Serialize)]
pub struct DividendHoldingEstimate {
    pub account_id: i64,
    pub account_name: String,
    pub symbol: String,
    pub lookup_symbol: String,
    pub quantity: f64,
    pub annual_dividend_per_share: f64,
    pub annual_income: f64,
    pub currency: String,
    pub estimated_annual: MoneyPair,
    pub yield_percent: Option<f64>,
    pub researched_on: String,
}

#[derive(Debug, Serialize)]
pub struct DividendSummary {
    pub estimated_annual: MoneyPair,
    pub recorded_last_12_months: MoneyPair,
    pub recorded_payment_count: usize,
    pub recorded_period_start: String,
    pub holdings: Vec<DividendHoldingEstimate>,
    pub position_count: usize,
    pub researched_positions: usize,
    pub dividend_positions: usize,
    pub unresolved_positions: usize,
    pub unresolved_symbols: Vec<String>,
    pub stale_positions: usize,
    pub research_error_count: usize,
    pub research_source: &'static str,
    pub research_as_of: Option<String>,
    pub currency_warning: bool,
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
    events: Option<YahooEvents>,
}

#[derive(Debug, Deserialize)]
struct YahooMeta {
    currency: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "previousClose")]
    previous_close: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct YahooEvents {
    #[serde(default)]
    dividends: HashMap<String, YahooDividend>,
}

#[derive(Debug, Deserialize)]
struct YahooDividend {
    amount: Option<f64>,
    date: Option<i64>,
}

struct YahooResearch {
    lookup_symbol: String,
    quote_currency: String,
    annual_dividend_per_share: f64,
    current_price: Option<f64>,
}

/// Estimate annual dividend income from current connected holdings and separately total actual
/// dividend payments found in the trailing twelve months of synced transaction history.
///
/// Market research is cached by ticker and holding currency for the UTC day. A forced refresh
/// bypasses that freshness check, while a failed lookup keeps any older cached result available.
#[tauri::command]
pub async fn get_dividend_summary(
    db: State<'_, AppDb>,
    force_refresh: bool,
) -> Result<DividendSummary, String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let (holdings, mut research) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (
            load_holdings(&conn).map_err(|e| e.to_string())?,
            load_research(&conn).map_err(|e| e.to_string())?,
        )
    };

    let mut requested: BTreeMap<ResearchKey, Option<f64>> = BTreeMap::new();
    for holding in holdings
        .iter()
        .filter(|holding| is_researchable_holding(holding))
    {
        let reference_price = holding
            .last_price
            .filter(|price| price.is_finite() && *price > 0.0);
        requested
            .entry(holding.key())
            .and_modify(|stored| {
                if stored.is_none() {
                    *stored = reference_price;
                }
            })
            .or_insert(reference_price);
    }
    let refresh_keys: Vec<(ResearchKey, Option<f64>)> = requested
        .into_iter()
        .filter(|(key, _)| {
            force_refresh
                || research
                    .get(key)
                    .map(|record| record.researched_on.as_str() < today.as_str())
                    .unwrap_or(true)
        })
        .collect();

    let mut fetched = Vec::new();
    let mut research_error_count = 0usize;
    if !refresh_keys.is_empty() {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("TrueNorth dividend research")
            .build()
            .map_err(|e| e.to_string())?;
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_RESEARCH_REQUESTS));
        let now = Utc::now();
        let mut jobs = JoinSet::new();

        for (key, reference_price) in refresh_keys {
            let client = client.clone();
            let semaphore = Arc::clone(&semaphore);
            let now = now;
            jobs.spawn(async move {
                let permit = semaphore.acquire_owned().await.map_err(|e| e.to_string())?;
                let result = fetch_research(&client, &key.0, &key.1, reference_price, now).await;
                drop(permit);
                Ok::<_, String>((key, result))
            });
        }

        while let Some(job) = jobs.join_next().await {
            match job {
                Ok(Ok((_key, Ok(record)))) => fetched.push(record),
                Ok(Ok((key, Err(error)))) => {
                    research_error_count += 1;
                    eprintln!(
                        "warning: dividend research failed for {} ({}): {}",
                        key.0, key.1, error
                    );
                }
                Ok(Err(error)) => {
                    research_error_count += 1;
                    eprintln!("warning: dividend research worker failed: {error}");
                }
                Err(error) => {
                    research_error_count += 1;
                    eprintln!("warning: dividend research task failed: {error}");
                }
            }
        }
    }

    if !fetched.is_empty() {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        for record in fetched {
            store_research(&conn, &record).map_err(|e| e.to_string())?;
            research.insert(record.key(), record);
        }
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    build_summary(&conn, &holdings, &research, &today, research_error_count)
        .map_err(|e| e.to_string())
}

fn load_holdings(conn: &Connection) -> rusqlite::Result<Vec<HoldingRow>> {
    let mut stmt = conn.prepare(
        "SELECT h.account_id, a.name, h.symbol, h.quantity, h.currency, h.last_price \
         FROM holdings h JOIN accounts a ON a.id = h.account_id \
         WHERE a.is_active = 1 AND h.quantity > 0 \
         ORDER BY a.name, h.symbol",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(HoldingRow {
                account_id: row.get(0)?,
                account_name: row.get(1)?,
                symbol: row.get(2)?,
                quantity: row.get(3)?,
                currency: row.get(4)?,
                last_price: row.get(5)?,
            })
        })?
        .collect();
    rows
}

fn load_research(conn: &Connection) -> rusqlite::Result<HashMap<ResearchKey, ResearchRecord>> {
    let mut stmt = conn.prepare(
        "SELECT symbol, holding_currency, lookup_symbol, quote_currency, \
                annual_dividend_per_share, current_price, researched_on \
         FROM dividend_research",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ResearchRecord {
            symbol: row.get(0)?,
            holding_currency: row.get(1)?,
            lookup_symbol: row.get(2)?,
            quote_currency: row.get(3)?,
            annual_dividend_per_share: row.get(4)?,
            current_price: row.get(5)?,
            researched_on: row.get(6)?,
        })
    })?;
    let mut records = HashMap::new();
    for row in rows {
        let record = row?;
        records.insert(record.key(), record);
    }
    Ok(records)
}

fn store_research(conn: &Connection, record: &ResearchRecord) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO dividend_research \
         (symbol, holding_currency, lookup_symbol, quote_currency, annual_dividend_per_share, \
          current_price, researched_on, source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'yahoo') \
         ON CONFLICT(symbol, holding_currency) DO UPDATE SET \
             lookup_symbol = excluded.lookup_symbol, \
             quote_currency = excluded.quote_currency, \
             annual_dividend_per_share = excluded.annual_dividend_per_share, \
             current_price = excluded.current_price, \
             researched_on = excluded.researched_on, \
             source = excluded.source",
        params![
            record.symbol,
            record.holding_currency,
            record.lookup_symbol,
            record.quote_currency,
            record.annual_dividend_per_share,
            record.current_price,
            record.researched_on,
        ],
    )?;
    Ok(())
}

async fn fetch_research(
    client: &Client,
    symbol: &str,
    holding_currency: &str,
    reference_price: Option<f64>,
    now: chrono::DateTime<Utc>,
) -> Result<ResearchRecord, String> {
    let candidates = research_symbols(symbol, holding_currency);
    if candidates.is_empty() {
        return Err("ticker is not researchable".into());
    }

    let mut errors = Vec::new();
    let mut results = Vec::new();
    for candidate in candidates {
        match fetch_yahoo_candidate(client, &candidate, now).await {
            Ok(result) => {
                if price_distance(&result, reference_price)
                    .map(|distance| distance <= 0.02)
                    .unwrap_or(false)
                {
                    return Ok(to_research_record(symbol, holding_currency, result, now));
                }
                results.push(result);
            }
            Err(error) => errors.push(error),
        }
    }

    let result = select_research_result(symbol, reference_price, results).map_err(|selection| {
        if errors.is_empty() {
            selection
        } else {
            format!("{selection}; {}", errors.join("; "))
        }
    })?;
    Ok(to_research_record(symbol, holding_currency, result, now))
}

/// Choose the listing whose quote price matches the broker's position price. Questrade reports the
/// account's preferred currency for every holding, so currency alone cannot distinguish a U.S.
/// stock such as AAPL from a Canadian CDR such as AAPL.TO. If no broker price is available, only an
/// unambiguous single Yahoo result is accepted rather than silently choosing the wrong security.
fn select_research_result(
    symbol: &str,
    reference_price: Option<f64>,
    results: Vec<YahooResearch>,
) -> Result<YahooResearch, String> {
    if results.is_empty() {
        return Err("no matching market listing found".into());
    }
    if results.len() == 1 {
        let result = results.into_iter().next().expect("one result exists");
        if let Some(reference_price) =
            reference_price.filter(|price| price.is_finite() && *price > 0.0)
        {
            return match price_distance(&result, Some(reference_price)) {
                Some(distance) if distance <= MAX_BROKER_PRICE_DISTANCE => Ok(result),
                Some(_) => Err(format!(
                    "{} price does not match broker price {reference_price:.2}",
                    result.lookup_symbol
                )),
                None => Err(format!(
                    "{} has no quote price to compare with broker price {reference_price:.2}",
                    result.lookup_symbol
                )),
            };
        }
        return Ok(result);
    }

    let Some(reference_price) = reference_price.filter(|price| price.is_finite() && *price > 0.0)
    else {
        return Err(format!(
            "{symbol} matches multiple market listings and has no broker price to disambiguate"
        ));
    };
    let mut scored: Vec<(f64, YahooResearch)> = results
        .into_iter()
        .filter_map(|result| {
            let distance = price_distance(&result, Some(reference_price))?;
            Some((distance, result))
        })
        .collect();
    if scored.is_empty() {
        return Err(format!(
            "{symbol} matches multiple market listings without comparable prices"
        ));
    }
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    if scored.len() == 1 {
        return if scored[0].0 <= MAX_BROKER_PRICE_DISTANCE {
            Ok(scored.remove(0).1)
        } else {
            Err(format!(
                "{symbol} has no listing close to broker price {reference_price:.2}"
            ))
        };
    }

    let best_distance = scored[0].0;
    let next_distance = scored[1].0;
    if best_distance <= MAX_BROKER_PRICE_DISTANCE && next_distance - best_distance >= 0.10 {
        return Ok(scored.remove(0).1);
    }
    Err(format!(
        "{symbol} market listing is ambiguous relative to broker price {reference_price:.2}"
    ))
}

fn price_distance(result: &YahooResearch, reference_price: Option<f64>) -> Option<f64> {
    let reference_price = reference_price.filter(|price| price.is_finite() && *price > 0.0)?;
    let quote_price = result
        .current_price
        .filter(|price| price.is_finite() && *price > 0.0)?;
    Some((quote_price - reference_price).abs() / reference_price)
}

fn to_research_record(
    symbol: &str,
    holding_currency: &str,
    result: YahooResearch,
    researched_at: chrono::DateTime<Utc>,
) -> ResearchRecord {
    ResearchRecord {
        symbol: normalize_symbol(symbol),
        holding_currency: holding_currency.trim().to_uppercase(),
        lookup_symbol: result.lookup_symbol,
        quote_currency: result.quote_currency,
        annual_dividend_per_share: result.annual_dividend_per_share,
        current_price: result.current_price,
        researched_on: researched_at.format("%Y-%m-%d").to_string(),
    }
}

async fn fetch_yahoo_candidate(
    client: &Client,
    lookup_symbol: &str,
    now: chrono::DateTime<Utc>,
) -> Result<YahooResearch, String> {
    let mut url = reqwest::Url::parse(YAHOO_CHART_URL).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "invalid Yahoo Finance base URL".to_string())?
        .push(lookup_symbol);
    let period_start = (now - ChronoDuration::days(370)).timestamp().to_string();
    let period_end = (now + ChronoDuration::days(1)).timestamp().to_string();
    url.query_pairs_mut()
        .append_pair("period1", &period_start)
        .append_pair("period2", &period_end)
        .append_pair("interval", "1d")
        .append_pair("events", "div");

    let response: YahooChartResponse = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(error) = response.chart.error {
        return Err(format!("Yahoo Finance error: {error}"));
    }
    let result = response
        .chart
        .result
        .and_then(|results| results.into_iter().next())
        .ok_or_else(|| "Yahoo Finance returned no result".to_string())?;
    let quote_currency = result
        .meta
        .currency
        .filter(|currency| !currency.trim().is_empty())
        .ok_or_else(|| "Yahoo Finance returned no quote currency".to_string())?
        .to_uppercase();
    let cutoff = (now - ChronoDuration::days(365)).timestamp();
    let now_ts = now.timestamp();
    let annual_dividend_per_share = result
        .events
        .unwrap_or_default()
        .dividends
        .values()
        .filter_map(|dividend| Some((dividend.amount?, dividend.date?)))
        .filter(|(amount, date)| {
            amount.is_finite() && *amount >= 0.0 && *date >= cutoff && *date <= now_ts
        })
        .map(|(amount, _)| amount)
        .sum();
    let current_price = result
        .meta
        .regular_market_price
        .or(result.meta.previous_close)
        .filter(|price| price.is_finite() && *price > 0.0);

    Ok(YahooResearch {
        lookup_symbol: lookup_symbol.to_string(),
        quote_currency,
        annual_dividend_per_share,
        current_price,
    })
}

fn build_summary(
    conn: &Connection,
    holdings: &[HoldingRow],
    research: &HashMap<ResearchKey, ResearchRecord>,
    today: &str,
    research_error_count: usize,
) -> rusqlite::Result<DividendSummary> {
    let usd_rates = load_usd_rates(conn)?;
    let mut estimated_annual = MoneyPair::default();
    let mut estimates = Vec::new();
    let mut researched_positions = 0usize;
    let mut dividend_positions = 0usize;
    let mut stale_positions = 0usize;
    let mut unresolved_symbols = BTreeSet::new();
    let mut research_as_of: Option<String> = None;
    let mut currency_warning = false;

    let researchable: Vec<&HoldingRow> = holdings
        .iter()
        .filter(|holding| is_researchable_holding(holding))
        .collect();
    for holding in &researchable {
        let key = holding.key();
        let Some(record) = research.get(&key) else {
            unresolved_symbols.insert(holding.symbol.clone());
            continue;
        };
        researched_positions += 1;
        if record.researched_on.as_str() < today {
            stale_positions += 1;
        }
        if research_as_of
            .as_ref()
            .map(|date| record.researched_on.as_str() > date.as_str())
            .unwrap_or(true)
        {
            research_as_of = Some(record.researched_on.clone());
        }

        let annual_income = holding.quantity * record.annual_dividend_per_share;
        let (usd, cad) = convert_balance(annual_income, &record.quote_currency, &usd_rates);
        if annual_income > 0.0 && (usd == 0.0 || cad == 0.0) {
            currency_warning = true;
        }
        estimated_annual.usd += usd;
        estimated_annual.cad += cad;
        if annual_income > 0.0 {
            dividend_positions += 1;
        }
        estimates.push(DividendHoldingEstimate {
            account_id: holding.account_id,
            account_name: holding.account_name.clone(),
            symbol: holding.symbol.clone(),
            lookup_symbol: record.lookup_symbol.clone(),
            quantity: holding.quantity,
            annual_dividend_per_share: record.annual_dividend_per_share,
            annual_income,
            currency: record.quote_currency.clone(),
            estimated_annual: MoneyPair { usd, cad },
            yield_percent: record.current_price.map(|price| {
                if price > 0.0 {
                    record.annual_dividend_per_share / price * 100.0
                } else {
                    0.0
                }
            }),
            researched_on: record.researched_on.clone(),
        });
    }
    estimates.sort_by(|a, b| {
        b.estimated_annual
            .usd
            .total_cmp(&a.estimated_annual.usd)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });

    let recorded_period_start = (NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .expect("internally generated date must be valid")
        - ChronoDuration::days(365))
    .format("%Y-%m-%d")
    .to_string();
    let (recorded_last_12_months, recorded_payment_count, recorded_currency_warning) =
        recorded_dividends(conn, &recorded_period_start, &usd_rates)?;
    currency_warning |= recorded_currency_warning;

    let unresolved_symbols: Vec<String> = unresolved_symbols.into_iter().collect();
    Ok(DividendSummary {
        estimated_annual,
        recorded_last_12_months,
        recorded_payment_count,
        recorded_period_start,
        holdings: estimates,
        position_count: researchable.len(),
        researched_positions,
        dividend_positions,
        unresolved_positions: unresolved_symbols.len(),
        unresolved_symbols,
        stale_positions,
        research_error_count,
        research_source: RESEARCH_SOURCE,
        research_as_of,
        currency_warning,
    })
}

fn recorded_dividends(
    conn: &Connection,
    since: &str,
    usd_rates: &HashMap<String, f64>,
) -> rusqlite::Result<(MoneyPair, usize, bool)> {
    let mut stmt = conn.prepare(
        "SELECT t.amount, t.currency \
         FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE a.is_active = 1 AND t.txn_date >= ?1 AND t.amount > 0 \
           AND a.account_type IN \
               ('brokerage', 'tfsa', 'rrsp', 'fhsa', '401k', 'ira', 'roth_ira', 'other') \
           AND ( \
               instr(lower(t.description), 'dividend') > 0 \
               OR instr(lower(COALESCE(t.memo, '')), 'dividend') > 0 \
               OR instr(lower(t.description), 'cash distribution') > 0 \
               OR instr(lower(t.description), 'income distribution') > 0 \
               OR trim(lower(t.description)) IN ('div', 'div dist', 'distribution') \
               OR lower(COALESCE(t.category, '')) IN ('dividend', 'dividends') \
           )",
    )?;
    let rows = stmt.query_map(params![since], |row| {
        Ok((row.get::<_, f64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut total = MoneyPair::default();
    let mut count = 0usize;
    let mut currency_warning = false;
    for row in rows {
        let (amount, currency) = row?;
        let (usd, cad) = convert_balance(amount, &currency, usd_rates);
        if usd == 0.0 || cad == 0.0 {
            currency_warning = true;
        }
        total.usd += usd;
        total.cad += cad;
        count += 1;
    }
    Ok((total, count, currency_warning))
}

fn normalize_symbol(symbol: &str) -> String {
    symbol.trim().trim_start_matches('$').to_uppercase()
}

fn is_researchable_holding(holding: &HoldingRow) -> bool {
    let symbol = normalize_symbol(&holding.symbol);
    let currency = holding.currency.trim().to_uppercase();
    !symbol.is_empty()
        && symbol != currency
        && symbol != "CASH"
        && !symbol.starts_with("CASH:")
        && !symbol.ends_with("=X")
        && symbol
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-^/".contains(character))
}

fn research_symbols(symbol: &str, holding_currency: &str) -> Vec<String> {
    let symbol = normalize_symbol(symbol);
    if symbol.is_empty() {
        return Vec::new();
    }
    let currency = holding_currency.trim().to_uppercase();
    let canadian_suffix = [".TO", ".V", ".CN", ".NE"]
        .iter()
        .any(|suffix| symbol.ends_with(suffix));
    let mut candidates = Vec::new();
    let mut push = |candidate: String| {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    push(symbol.clone());
    if symbol.contains('.') && !canadian_suffix {
        push(symbol.replace('.', "-"));
    }
    if currency == "CAD" && !canadian_suffix {
        push(format!("{symbol}.TO"));
        push(format!("{symbol}.V"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::apply_schema;
    use chrono::TimeZone;

    #[test]
    fn canadian_symbols_research_the_reported_ticker_before_synthesized_listings() {
        assert_eq!(
            research_symbols("XEI", "CAD"),
            vec!["XEI", "XEI.TO", "XEI.V"]
        );
        assert_eq!(research_symbols("VAB.TO", "CAD"), vec!["VAB.TO"]);
        assert_eq!(research_symbols("BRK.B", "USD"), vec!["BRK.B", "BRK-B"]);
    }

    #[test]
    fn broker_price_disambiguates_us_stock_from_canadian_cdr() {
        let us = YahooResearch {
            lookup_symbol: "AAPL".into(),
            quote_currency: "USD".into(),
            annual_dividend_per_share: 1.06,
            current_price: Some(309.0),
        };
        let cdr = YahooResearch {
            lookup_symbol: "AAPL.TO".into(),
            quote_currency: "CAD".into(),
            annual_dividend_per_share: 0.15,
            current_price: Some(45.0),
        };

        let selected = select_research_result("AAPL", Some(308.5), vec![cdr, us]).unwrap();
        assert_eq!(selected.lookup_symbol, "AAPL");
    }

    #[test]
    fn ambiguous_listings_without_a_broker_price_are_not_guessed() {
        let us = YahooResearch {
            lookup_symbol: "AAPL".into(),
            quote_currency: "USD".into(),
            annual_dividend_per_share: 1.06,
            current_price: Some(309.0),
        };
        let cdr = YahooResearch {
            lookup_symbol: "AAPL.TO".into(),
            quote_currency: "CAD".into(),
            annual_dividend_per_share: 0.15,
            current_price: Some(45.0),
        };

        assert!(select_research_result("AAPL", None, vec![cdr, us]).is_err());
    }

    #[test]
    fn sole_cross_listing_is_rejected_when_its_price_disagrees_with_the_broker() {
        let cdr = YahooResearch {
            lookup_symbol: "AAPL.TO".into(),
            quote_currency: "CAD".into(),
            annual_dividend_per_share: 0.15,
            current_price: Some(45.0),
        };

        assert!(select_research_result("AAPL", Some(309.0), vec![cdr]).is_err());
    }

    #[test]
    fn summary_combines_position_research_and_recorded_payments_without_double_counting() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2026-08-22')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts \
             (name, institution, account_type, currency, jurisdiction, connector_kind) \
             VALUES ('US brokerage', 'Test', 'brokerage', 'USD', 'US', 'snaptrade'), \
                    ('TFSA', 'Test', 'tfsa', 'CAD', 'CA', 'simplefin'), \
                    ('RESP', 'Test', 'other', 'CAD', 'CA', 'simplefin')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO holdings (account_id, symbol, quantity, currency) \
             VALUES (1, 'AAPL', 10.0, 'CAD'), (2, 'XEI.TO', 20.0, 'CAD')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dividend_research \
             (symbol, holding_currency, lookup_symbol, quote_currency, annual_dividend_per_share, \
              current_price, researched_on) \
             VALUES ('AAPL', 'CAD', 'AAPL', 'USD', 1.0, 100.0, '2026-08-22'), \
                    ('XEI.TO', 'CAD', 'XEI.TO', 'CAD', 2.0, 20.0, '2026-08-22')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transactions \
             (account_id, txn_date, description, amount, currency, connector_ref) \
             VALUES (1, '2026-07-01', 'AAPL dividend', 5.0, 'USD', 'div-1'), \
                    (2, '2026-07-02', 'Cash distribution', 10.0, 'CAD', 'div-2'), \
                    (3, '2026-07-03', 'Fund dividend', 4.0, 'CAD', 'div-3'), \
                    (1, '2026-07-04', 'Deposit', 999.0, 'USD', 'deposit')",
            [],
        )
        .unwrap();

        let holdings = load_holdings(&conn).unwrap();
        let research = load_research(&conn).unwrap();
        let summary = build_summary(&conn, &holdings, &research, "2026-08-22", 0).unwrap();

        assert_eq!(summary.position_count, 2);
        assert_eq!(summary.researched_positions, 2);
        assert_eq!(summary.dividend_positions, 2);
        assert_eq!(summary.recorded_payment_count, 3);
        assert_eq!(
            summary.estimated_annual,
            MoneyPair {
                usd: 30.0,
                cad: 60.0
            }
        );
        assert_eq!(
            summary.recorded_last_12_months,
            MoneyPair {
                usd: 12.0,
                cad: 24.0
            }
        );
    }

    #[test]
    fn yahoo_dividends_are_limited_to_the_trailing_twelve_months() {
        let now = Utc.with_ymd_and_hms(2026, 8, 22, 0, 0, 0).unwrap();
        let body = serde_json::json!({
            "chart": {
                "result": [{
                    "meta": {
                        "currency": "USD",
                        "regularMarketPrice": 100.0
                    },
                    "events": {
                        "dividends": {
                            "recent": {
                                "amount": 0.5,
                                "date": (now - ChronoDuration::days(90)).timestamp()
                            },
                            "old": {
                                "amount": 9.0,
                                "date": (now - ChronoDuration::days(366)).timestamp()
                            }
                        }
                    }
                }],
                "error": null
            }
        });
        let parsed: YahooChartResponse = serde_json::from_value(body).unwrap();
        let result = parsed.chart.result.unwrap().remove(0);
        let cutoff = (now - ChronoDuration::days(365)).timestamp();
        let total: f64 = result
            .events
            .unwrap()
            .dividends
            .values()
            .filter_map(|dividend| Some((dividend.amount?, dividend.date?)))
            .filter(|(_, date)| *date >= cutoff && *date <= now.timestamp())
            .map(|(amount, _)| amount)
            .sum();
        assert_eq!(total, 0.5);
    }
}
