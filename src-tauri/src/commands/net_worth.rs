use serde::Serialize;
use std::collections::{HashMap, HashSet};
use rusqlite::Connection;
use tauri::State;

use crate::db::AppDb;
use crate::fx::{load_latest_rates, load_usd_rates};

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
    pub allocation: NetWorthAllocation,
    pub usd_cad_rate: Option<f64>,
    pub cad_usd_rate: Option<f64>,
    pub rate_date: Option<String>,
}

/// A money figure carried in both reporting currencies so the frontend can render either side
/// of the USD/CAD toggle without a second round-trip.
#[derive(Debug, Serialize, PartialEq, Default, Clone, Copy)]
pub struct MoneyPair {
    pub usd: f64,
    pub cad: f64,
}

impl MoneyPair {
    fn add(&mut self, usd: f64, cad: f64) {
        self.usd += usd;
        self.cad += cad;
    }

    fn minus(self, other: MoneyPair) -> MoneyPair {
        MoneyPair {
            usd: self.usd - other.usd,
            cad: self.cad - other.cad,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct NetWorthAllocation {
    pub investments: MoneyPair,
    pub savings: MoneyPair,
    /// Signed account debts are negated here, so a positive figure means money owed.
    pub liabilities: MoneyPair,
    /// Unknown assets still contribute to total wealth, but must not be labeled as cash.
    pub unclassified: MoneyPair,
}

/// How an account contributes to the "Anxiety Buffer" split. `Liquid` is spendable cash
/// (chequing/savings) — the balance that drops after paying a credit card and triggers panic.
/// `Invested` is the long-horizon pile that usually offsets it. Liabilities and unknown assets
/// still count toward net worth but not toward either side of that split.
#[derive(Debug, Clone, Copy, PartialEq)]
enum AccountClass {
    Liquid,
    Invested,
    Liability,
    Other,
}

pub(crate) fn is_investment_account_type(account_type: &str) -> bool {
    matches!(
        account_type.trim().to_ascii_lowercase().as_str(),
        "brokerage"
            | "investment"
            | "retirement"
            | "crypto"
            | "ira"
            | "roth_ira"
            | "rrsp"
            | "tfsa"
            | "resp"
            | "fhsa"
            | "lira"
            | "rrif"
            | "rlif"
            | "401k"
            | "pension"
    )
}

fn account_class(account_type: &str, name: &str, institution: &str) -> AccountClass {
    let normalized = account_type.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "credit" | "credit_card" | "loan" | "mortgage" | "line_of_credit" | "debt" | "liability"
    ) {
        return AccountClass::Liability;
    }
    if is_investment_account_type(&normalized) {
        return AccountClass::Invested;
    }

    let name = name.to_ascii_lowercase();
    let institution = institution.to_ascii_lowercase();
    if name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        matches!(word, "loan" | "loans" | "mortgage")
    }) || name.contains("credit card") || name.contains("line of credit")
    {
        return AccountClass::Liability;
    }

    // Aggregators can default stock plans and workplace retirement accounts to chequing/savings.
    let investment_institution = [
        "questrade", "robinhood", "sun life", "sunlife", "morgan stanley at work",
        "stockplan", "shareworks",
    ]
    .iter()
    .any(|keyword| institution.contains(*keyword));
    let investment_name = [
        "brokerage", "invest", "retirement", "pension", "stock plan", "stockplan",
        "stock award", "equity award", "employee stock", "employee share",
    ]
    .iter()
    .any(|keyword| name.contains(*keyword))
        || name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
            is_investment_account_type(word) || matches!(word, "rsu" | "rsus" | "espp")
        });
    if investment_institution || investment_name {
        return AccountClass::Invested;
    }

    if matches!(
        normalized.as_str(),
        "chequing" | "checking" | "savings" | "cash" | "money_market" | "certificate_of_deposit"
    ) || name.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        matches!(word, "chequing" | "checking" | "savings" | "cash")
    }) {
        return AccountClass::Liquid;
    }
    AccountClass::Other
}

/// Compute the current net worth across all active accounts.
///
/// Uses the most recent FX rate in the database. If no rate is available,
/// values in the non-native currency are returned as 0.
#[tauri::command]
pub fn get_net_worth(db: State<AppDb>) -> Result<NetWorthResponse, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    compute_net_worth(&conn)
}

pub(crate) fn compute_net_worth(conn: &Connection) -> Result<NetWorthResponse, String> {
    let rates = load_latest_rates(&conn).map_err(|e| e.to_string())?;
    let (usd_cad, cad_usd, rate_date) = match rates {
        Some((u, c, d)) => (Some(u), Some(c), Some(d)),
        None => (None, None, None),
    };
    let usd_rates = load_usd_rates(&conn).map_err(|e| e.to_string())?;

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
    let mut allocation = NetWorthAllocation::default();

    for (id, name, institution, account_type, jurisdiction, currency, balance_opt, snapshot_date) in
        account_rows
    {
        let balance = balance_opt.unwrap_or(0.0);

        let (balance_usd, balance_cad) = convert_balance(balance, &currency, &usd_rates);
        total_usd += balance_usd;
        total_cad += balance_cad;

        let class = if balance < 0.0 {
            AccountClass::Liability
        } else {
            account_class(&account_type, &name, &institution)
        };
        match class {
            AccountClass::Invested => allocation.investments.add(balance_usd, balance_cad),
            AccountClass::Liquid => allocation.savings.add(balance_usd, balance_cad),
            AccountClass::Liability => allocation.liabilities.add(-balance_usd, -balance_cad),
            AccountClass::Other => allocation.unclassified.add(balance_usd, balance_cad),
        }

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
        allocation,
        usd_cad_rate: usd_cad,
        cad_usd_rate: cad_usd,
        rate_date,
    })
}

/// Convert a balance in `currency` to both USD and CAD using a USD-pivot rate map
/// (`currency -> units per 1 USD`, with `USD = 1.0`).
///
/// If we have no rate for `currency`, it contributes 0 (we can't place it on the books yet —
/// refreshing FX will pick the currency up). CAD falls back to 0 only when no USD→CAD rate
/// is stored.
pub(crate) fn convert_balance(
    balance: f64,
    currency: &str,
    usd_rates: &HashMap<String, f64>,
) -> (f64, f64) {
    let usd = if currency == "USD" {
        balance
    } else {
        match usd_rates.get(currency) {
            Some(rate) if *rate != 0.0 => balance / rate,
            _ => return (0.0, 0.0),
        }
    };

    let cad = usd_rates.get("CAD").map(|r| usd * r).unwrap_or(0.0);
    (usd, cad)
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
/// For each date on which any account has a *real* balance snapshot, every account's most
/// recent balance *as of that date* is carried forward (accounts contribute 0 before
/// their first snapshot), converted with the latest stored FX rate, and summed. Using
/// a single consistent FX rate keeps the trend driven by balances, not FX noise.
///
/// Only observed snapshots count — manual entries and connector syncs. Nothing is inferred:
/// balances are never reconstructed from transactions, so an internal transfer (which lowers
/// one account's real balance and raises another's) nets out instead of masquerading as growth.
#[tauri::command]
pub fn get_net_worth_history(db: State<AppDb>) -> Result<Vec<NetWorthHistoryPoint>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    compute_net_worth_history(&conn).map_err(|e| e.to_string())
}

pub(crate) fn compute_net_worth_history(
    conn: &Connection,
) -> rusqlite::Result<Vec<NetWorthHistoryPoint>> {
    let usd_rates = load_usd_rates(conn)?;

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

    // All *real* snapshots for active accounts, oldest first. Reconstructed rows are excluded so
    // the trend reflects only observed balances.
    let snapshots: Vec<(i64, String, f64)> = {
        let mut stmt = conn.prepare(
            "SELECT bs.account_id, bs.snapshot_date, bs.balance \
             FROM balance_snapshots bs \
             JOIN accounts a ON a.id = bs.account_id \
             WHERE a.is_active = 1 AND bs.source != 'backfill' \
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
            let (usd, cad) = convert_balance(*balance, currency, &usd_rates);
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

// ---------------------------------------------------------------------------
// Net-worth delta — the "Anxiety Buffer"
// ---------------------------------------------------------------------------

/// The change in net worth since the previous snapshot date, split so the UI can reassure the
/// user that a dip in spendable cash hasn't actually shrunk their net worth.
#[derive(Debug, Serialize, PartialEq)]
pub struct NetWorthDelta {
    /// The most recent snapshot date the totals reflect (YYYY-MM-DD), or None when there's no data.
    pub current_date: Option<String>,
    /// The prior snapshot date the deltas are measured against, or None when only one date exists.
    pub previous_date: Option<String>,
    /// Current totals.
    pub total: MoneyPair,
    pub liquid: MoneyPair,
    pub invested: MoneyPair,
    /// Change versus `previous_date`, measured like-for-like over only the accounts that already
    /// existed on `previous_date`. Accounts added afterward don't count as a gain. Zero when there
    /// is no prior date to compare against.
    pub total_delta: MoneyPair,
    pub liquid_delta: MoneyPair,
    pub invested_delta: MoneyPair,
    /// True when a prior snapshot date exists, i.e. the deltas are meaningful.
    pub has_previous: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct ClassBreakdown {
    total: MoneyPair,
    liquid: MoneyPair,
    invested: MoneyPair,
}

/// Current net worth split into spendable cash vs. investments, plus the delta against the
/// previous snapshot date. Powers the dashboard's reassurance line ("cash down, net worth up").
#[tauri::command]
pub fn get_net_worth_delta(db: State<AppDb>) -> Result<NetWorthDelta, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    compute_net_worth_delta(&conn).map_err(|e| e.to_string())
}

fn compute_net_worth_delta(conn: &Connection) -> rusqlite::Result<NetWorthDelta> {
    let usd_rates = load_usd_rates(conn)?;
    let meta = load_account_class_meta(conn)?;
    let series = compute_carried_account_series(conn)?;

    let (current_date, current_balances) = match series.last() {
        Some((date, balances)) => (Some(date.clone()), balances.clone()),
        None => (None, HashMap::new()),
    };

    // Current totals reflect every account's latest balance.
    let current = breakdown(&current_balances, &meta, &usd_rates, None);

    let (previous_date, total_delta, liquid_delta, invested_delta, has_previous) =
        if series.len() >= 2 {
            let (previous_date, previous_balances) = &series[series.len() - 2];

            // Compare like-for-like: only accounts that already had a snapshot on or before the
            // previous date. An account whose first snapshot lands on the current date is "coming
            // online" (0 -> full balance), which would otherwise masquerade as a huge gain.
            let cohort: HashSet<i64> = previous_balances.keys().copied().collect();
            let previous = breakdown(previous_balances, &meta, &usd_rates, None);
            let current_cohort = breakdown(&current_balances, &meta, &usd_rates, Some(&cohort));

            (
                Some(previous_date.clone()),
                current_cohort.total.minus(previous.total),
                current_cohort.liquid.minus(previous.liquid),
                current_cohort.invested.minus(previous.invested),
                true,
            )
        } else {
            (
                None,
                MoneyPair::default(),
                MoneyPair::default(),
                MoneyPair::default(),
                false,
            )
        };

    Ok(NetWorthDelta {
        current_date,
        previous_date,
        total: current.total,
        liquid: current.liquid,
        invested: current.invested,
        total_delta,
        liquid_delta,
        invested_delta,
        has_previous,
    })
}

/// active account_id -> (currency, class).
fn load_account_class_meta(
    conn: &Connection,
) -> rusqlite::Result<HashMap<i64, (String, AccountClass)>> {
    let mut meta: HashMap<i64, (String, AccountClass)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, currency, account_type, name, institution FROM accounts WHERE is_active = 1",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id, currency, account_type, name, institution) = row?;
        meta.insert(id, (currency, account_class(&account_type, &name, &institution)));
    }
    Ok(meta)
}

/// Sum carried balances into total/liquid/invested buckets (USD + CAD). When `restrict` is set,
/// only the listed accounts contribute — used to measure a like-for-like delta over a fixed cohort.
fn breakdown(
    balances: &HashMap<i64, f64>,
    meta: &HashMap<i64, (String, AccountClass)>,
    usd_rates: &HashMap<String, f64>,
    restrict: Option<&HashSet<i64>>,
) -> ClassBreakdown {
    let mut bd = ClassBreakdown::default();
    for (account_id, balance) in balances {
        if let Some(cohort) = restrict {
            if !cohort.contains(account_id) {
                continue;
            }
        }
        let (currency, class) = match meta.get(account_id) {
            Some((c, cls)) => (c.as_str(), *cls),
            None => ("USD", AccountClass::Other),
        };
        let (usd, cad) = convert_balance(*balance, currency, usd_rates);
        bd.total.add(usd, cad);
        match class {
            AccountClass::Liquid => bd.liquid.add(usd, cad),
            AccountClass::Invested => bd.invested.add(usd, cad),
            AccountClass::Liability | AccountClass::Other => {}
        }
    }
    bd
}

/// Walk the snapshot dates (carrying each account's latest balance forward, exactly like the
/// history series) and capture every account's carried balance at each date. Accounts contribute
/// nothing before their first snapshot.
fn compute_carried_account_series(
    conn: &Connection,
) -> rusqlite::Result<Vec<(String, HashMap<i64, f64>)>> {
    let snapshots: Vec<(i64, String, f64)> = {
        let mut stmt = conn.prepare(
            "SELECT bs.account_id, bs.snapshot_date, bs.balance \
             FROM balance_snapshots bs \
             JOIN accounts a ON a.id = bs.account_id \
             WHERE a.is_active = 1 AND bs.source != 'backfill' \
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

    let mut current: HashMap<i64, f64> = HashMap::new();
    let mut points: Vec<(String, HashMap<i64, f64>)> = Vec::new();
    let mut idx = 0;
    while idx < snapshots.len() {
        let date = snapshots[idx].1.clone();
        while idx < snapshots.len() && snapshots[idx].1 == date {
            current.insert(snapshots[idx].0, snapshots[idx].2);
            idx += 1;
        }
        points.push((date, current.clone()));
    }

    Ok(points)
}

/// Like-for-like net-worth movement over the snapshot period nearest the requested window.
///
/// The baseline is the latest observed snapshot date on or before `window_days` before the current
/// snapshot. If the database has less history, its earliest prior date is used and `window_days`
/// reports the shorter observed period. Accounts first seen after the baseline are excluded from
/// both the change and the transaction reconciliation so linking an existing account cannot look
/// like new savings.
#[derive(Debug, PartialEq)]
pub(crate) struct NetWorthPeriod {
    pub since: String,
    pub through: String,
    pub window_days: i64,
    pub account_ids: HashSet<i64>,
    pub change: MoneyPair,
}

pub(crate) fn compute_net_worth_period(
    conn: &Connection,
    window_days: i64,
) -> rusqlite::Result<Option<NetWorthPeriod>> {
    let series = compute_carried_account_series(conn)?;
    if series.len() < 2 {
        return Ok(None);
    }

    let (through_text, current_balances) = &series[series.len() - 1];
    let Ok(through) = chrono::NaiveDate::parse_from_str(through_text, "%Y-%m-%d") else {
        return Ok(None);
    };
    let target = through - chrono::Duration::days(window_days.clamp(1, 3650));
    let prior = &series[..series.len() - 1];

    let baseline_index = prior
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, (date, _))| {
            chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .ok()
                .filter(|parsed| *parsed <= target)
                .map(|_| index)
        })
        .or_else(|| {
            prior.iter().enumerate().find_map(|(index, (date, _))| {
                chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .ok()
                    .map(|_| index)
            })
        });
    let Some(baseline_index) = baseline_index else {
        return Ok(None);
    };

    let (since_text, baseline_balances) = &prior[baseline_index];
    let Ok(since) = chrono::NaiveDate::parse_from_str(since_text, "%Y-%m-%d") else {
        return Ok(None);
    };
    let observed_days = (through - since).num_days();
    if observed_days <= 0 {
        return Ok(None);
    }

    let account_ids: HashSet<i64> = baseline_balances.keys().copied().collect();
    let meta = load_account_class_meta(conn)?;
    let usd_rates = load_usd_rates(conn)?;
    let baseline = breakdown(baseline_balances, &meta, &usd_rates, Some(&account_ids));
    let current = breakdown(current_balances, &meta, &usd_rates, Some(&account_ids));

    Ok(Some(NetWorthPeriod {
        since: since_text.clone(),
        through: through_text.clone(),
        window_days: observed_days,
        account_ids,
        change: current.total.minus(baseline.total),
    }))
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

    fn add_typed_account(conn: &Connection, name: &str, currency: &str, account_type: &str) -> i64 {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES (?1, 'Inst', ?3, ?2, 'CA')",
            rusqlite::params![name, currency, account_type],
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
    fn allocation_recognizes_investments_cash_and_debt() {
        for account_type in [
            "brokerage", "tfsa", "rrsp", "fhsa", "401k", "ira", "roth_ira", "crypto",
            "retirement", "resp", "lira", "rrif", "pension",
        ] {
            assert_eq!(
                account_class(account_type, "Account", "Institution"),
                AccountClass::Invested,
                "{account_type}",
            );
        }
        for (account_type, name, institution, expected) in [
            ("other", "RESP", "Questrade", AccountClass::Invested),
            ("chequing", "Individual", "Robinhood", AccountClass::Invested),
            ("chequing", "Microsoft Stock Awards", "Morgan Stanley", AccountClass::Invested),
            ("other", "Workplace plan", "Morgan Stanley at Work", AccountClass::Invested),
            ("savings", "Group savings", "Sun Life Financial", AccountClass::Invested),
            ("chequing", "Workplace plan", "SUNLIFE", AccountClass::Invested),
            ("other", "Employee Share Purchase Plan", "Employer", AccountClass::Invested),
            ("other", "RSU awards", "Employer", AccountClass::Invested),
            ("savings", "Retirement savings", "Institution", AccountClass::Invested),
            ("chequing", "Everyday account", "Scotiabank", AccountClass::Liquid),
            ("savings", "Savings", "Bank", AccountClass::Liquid),
            ("other", "Cash reserve", "Bank", AccountClass::Liquid),
            ("checking", "Everyday account", "Morgan Stanley", AccountClass::Liquid),
            ("credit", "Rewards", "Sun Life", AccountClass::Liability),
            ("loan", "Account", "Institution", AccountClass::Liability),
            ("other", "Home mortgage", "Bank", AccountClass::Liability),
            ("other", "Line of credit", "Bank", AccountClass::Liability),
            ("brokerage", "Mortgage fund", "Broker", AccountClass::Invested),
            ("other", "Home value", "Manual", AccountClass::Other),
        ] {
            assert_eq!(account_class(account_type, name, institution), expected, "{name}");
        }
    }

    #[test]
    fn allocation_groups_every_active_account_using_latest_converted_balances() {
        let conn = setup();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2025-01-01')",
            [],
        )
        .unwrap();

        for (name, institution, kind, currency, balance) in [
            ("TFSA", "Questrade", "tfsa", "CAD", 2000.0),
            ("Individual", "Robinhood", "brokerage", "USD", 3000.0),
            ("Microsoft Stock Awards", "Morgan Stanley", "chequing", "USD", 4000.0),
            ("Group savings", "Sun Life", "savings", "CAD", 10000.0),
            ("RESP", "Questrade", "other", "CAD", 6000.0),
            ("Checking", "Bank", "chequing", "USD", 200.0),
            ("Savings", "Bank", "savings", "CAD", 600.0),
            ("Credit card", "Bank", "credit", "USD", -100.0),
            ("Home mortgage", "Bank", "other", "CAD", -1000.0),
            ("Overdraft", "Bank", "chequing", "CAD", -50.0),
            ("Margin", "Broker", "brokerage", "USD", -200.0),
        ] {
            let id = add_typed_account(&conn, name, currency, kind);
            conn.execute(
                "UPDATE accounts SET institution = ?1 WHERE id = ?2",
                rusqlite::params![institution, id],
            )
            .unwrap();
            add_snapshot(&conn, id, "2025-01-01", 99999.0, currency);
            add_snapshot(&conn, id, "2025-01-02", balance, currency);
        }
        let inactive = add_typed_account(&conn, "Inactive", "USD", "brokerage");
        add_snapshot(&conn, inactive, "2025-01-03", 90000.0, "USD");
        conn.execute("UPDATE accounts SET is_active = 0 WHERE id = ?1", [inactive]).unwrap();
        let empty = add_typed_account(&conn, "Not synced yet", "USD", "savings");

        let nw = compute_net_worth(&conn).unwrap();
        assert_eq!(nw.accounts.len(), 12);
        assert!(nw.accounts.iter().all(|account| account.account_id != inactive));
        assert_eq!(
            nw.accounts.iter().find(|account| account.account_id == empty).unwrap().balance,
            0.0,
        );
        assert_eq!(nw.allocation, NetWorthAllocation {
            investments: MoneyPair { usd: 16000.0, cad: 32000.0 },
            savings: MoneyPair { usd: 500.0, cad: 1000.0 },
            liabilities: MoneyPair { usd: 825.0, cad: 1650.0 },
            unclassified: MoneyPair::default(),
        });
        assert_eq!(nw.total_usd, 15675.0);
        assert_eq!(nw.total_cad, 31350.0);
        assert_eq!(
            nw.total_usd,
            nw.allocation.investments.usd + nw.allocation.savings.usd
                - nw.allocation.liabilities.usd,
        );
        let json = serde_json::to_value(&nw).unwrap();
        assert_eq!(json["allocation"]["investments"]["cad"], 32000.0);
        assert_eq!(json["allocation"]["liabilities"]["usd"], 825.0);
    }

    #[test]
    fn allocation_keeps_unknown_assets_out_of_cash_and_offsets_credit_balances() {
        let conn = setup();
        let property = add_typed_account(&conn, "Home value", "USD", "other");
        let credit = add_typed_account(&conn, "Credit card", "USD", "credit");
        let refund = add_typed_account(&conn, "Credit balance", "USD", "credit");
        add_snapshot(&conn, property, "2025-01-01", 10000.0, "USD");
        add_snapshot(&conn, credit, "2025-01-01", -200.0, "USD");
        add_snapshot(&conn, refund, "2025-01-01", 50.0, "USD");

        let nw = compute_net_worth(&conn).unwrap();
        assert_eq!(nw.allocation.savings, MoneyPair::default());
        assert_eq!(nw.allocation.investments, MoneyPair::default());
        assert_eq!(nw.allocation.unclassified.usd, 10000.0);
        assert_eq!(nw.allocation.liabilities.usd, 150.0);
        assert_eq!(nw.total_usd, 9850.0);
    }

    #[test]
    fn allocation_is_zero_without_balances() {
        let conn = setup();
        assert_eq!(compute_net_worth(&conn).unwrap().allocation, NetWorthAllocation::default());
        add_typed_account(&conn, "Savings", "USD", "savings");
        add_typed_account(&conn, "Investments", "USD", "brokerage");
        add_typed_account(&conn, "Credit card", "USD", "credit");
        assert_eq!(compute_net_worth(&conn).unwrap().allocation, NetWorthAllocation::default());
    }

    #[test]
    fn allocation_uses_the_same_fx_and_missing_rate_behavior_as_total_wealth() {
        let conn = setup();
        for (currency, rate) in [("CAD", 1.30), ("JMD", 155.0)] {
            conn.execute(
                "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
                 VALUES ('USD', ?1, ?2, '2025-01-01')",
                rusqlite::params![currency, rate],
            )
            .unwrap();
        }
        let cash = add_typed_account(&conn, "Chequing", "JMD", "chequing");
        let debt = add_typed_account(&conn, "Credit card", "JMD", "credit");
        let no_rate = add_typed_account(&conn, "Brokerage", "GBP", "brokerage");
        add_snapshot(&conn, cash, "2025-01-01", 15500.0, "JMD");
        add_snapshot(&conn, debt, "2025-01-01", -1550.0, "JMD");
        add_snapshot(&conn, no_rate, "2025-01-01", 500.0, "GBP");

        let nw = compute_net_worth(&conn).unwrap();
        assert_eq!(nw.allocation.savings, MoneyPair { usd: 100.0, cad: 130.0 });
        assert_eq!(nw.allocation.liabilities, MoneyPair { usd: 10.0, cad: 13.0 });
        assert_eq!(nw.allocation.investments, MoneyPair::default());
        assert_eq!(nw.total_usd, 90.0);
        assert_eq!(nw.total_cad, 117.0);
    }

    #[test]
    fn delta_uses_the_same_stock_plan_and_retirement_classification() {
        let conn = setup();
        let stock_plan = add_typed_account(&conn, "Microsoft Stock Awards", "USD", "chequing");
        let retirement = add_typed_account(&conn, "Workplace plan", "USD", "savings");
        conn.execute(
            "UPDATE accounts SET institution = 'Sun Life' WHERE id = ?1",
            [retirement],
        )
        .unwrap();
        for id in [stock_plan, retirement] {
            add_snapshot(&conn, id, "2025-01-01", 1000.0, "USD");
            add_snapshot(&conn, id, "2025-01-02", 1200.0, "USD");
        }

        let delta = compute_net_worth_delta(&conn).unwrap();
        assert_eq!(delta.invested.usd, 2400.0);
        assert_eq!(delta.invested_delta.usd, 400.0);
        assert_eq!(delta.liquid, MoneyPair::default());
        assert_eq!(delta.liquid_delta, MoneyPair::default());
        assert_eq!(delta.total_delta.usd, 400.0);
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

    #[test]
    fn history_ignores_inferred_backfill_snapshots() {
        // Only observed balances count. A leftover reconstructed row (source = 'backfill') from a
        // prior app version must not appear in the trend, so nothing on the chart is inferred.
        let conn = setup();
        let acct = add_account(&conn, "CA Savings", "CAD");
        add_snapshot(&conn, acct, "2025-03-01", 200.0, "CAD");
        conn.execute(
            "INSERT INTO balance_snapshots (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, '2025-01-01', 50.0, 'CAD', 'backfill')",
            rusqlite::params![acct],
        )
        .unwrap();

        let series = compute_net_worth_history(&conn).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].date, "2025-03-01");

        // The delta series is likewise driven only by real snapshots, so a lone real snapshot has
        // no prior date to compare against.
        let d = compute_net_worth_delta(&conn).unwrap();
        assert!(!d.has_previous);
        assert_eq!(d.current_date.as_deref(), Some("2025-03-01"));
    }

    #[test]
    fn convert_balance_pivots_any_currency_through_usd() {
        // 1 USD = 1.30 CAD = 155 JMD.
        let mut rates = HashMap::new();
        rates.insert("USD".to_string(), 1.0);
        rates.insert("CAD".to_string(), 1.30);
        rates.insert("JMD".to_string(), 155.0);

        // USD passes through; CAD column applies the USD→CAD rate.
        assert_eq!(convert_balance(100.0, "USD", &rates), (100.0, 130.0));

        // CAD round-trips exactly back to itself.
        let (usd, cad) = convert_balance(130.0, "CAD", &rates);
        assert!((usd - 100.0).abs() < 1e-9);
        assert!((cad - 130.0).abs() < 1e-9);

        // JMD (Scotiabank Jamaica): 15_500 JMD = 100 USD = 130 CAD.
        let (usd, cad) = convert_balance(15_500.0, "JMD", &rates);
        assert!((usd - 100.0).abs() < 1e-9);
        assert!((cad - 130.0).abs() < 1e-9);

        // A liability (e.g. a credit card SimpleFIN reports as negative) subtracts.
        let (usd, _) = convert_balance(-15_500.0, "JMD", &rates);
        assert!((usd + 100.0).abs() < 1e-9);

        // A currency with no stored rate can't be placed yet → contributes 0.
        assert_eq!(convert_balance(500.0, "GBP", &rates), (0.0, 0.0));
    }

    #[test]
    fn history_totals_include_a_third_currency() {
        let conn = setup();
        // USD→CAD = 1.30, USD→JMD = 155.
        for (to, rate) in [("CAD", 1.30_f64), ("JMD", 155.0)] {
            conn.execute(
                "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
                 VALUES ('USD', ?1, ?2, '2025-01-01')",
                rusqlite::params![to, rate],
            )
            .unwrap();
        }

        let jmd = add_account(&conn, "Scotiabank Jamaica", "JMD");
        add_snapshot(&conn, jmd, "2025-01-01", 15_500.0, "JMD");

        let series = compute_net_worth_history(&conn).unwrap();
        assert_eq!(series.len(), 1);
        // 15_500 JMD = 100 USD = 130 CAD.
        assert!((series[0].total_usd - 100.0).abs() < 1e-9);
        assert!((series[0].total_cad - 130.0).abs() < 1e-9);
    }

    #[test]
    fn delta_splits_cash_from_investments_and_compares_last_two_dates() {
        let conn = setup();
        // USD -> CAD = 2.0 so the CAD mirror is just double the USD figure.
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2025-01-01')",
            [],
        )
        .unwrap();

        let chequing = add_typed_account(&conn, "Chase Checking", "USD", "chequing");
        let brokerage = add_typed_account(&conn, "Robinhood", "USD", "brokerage");

        // Day 1: cash 1000, invested 500 -> net worth 1500.
        add_snapshot(&conn, chequing, "2025-01-01", 1000.0, "USD");
        add_snapshot(&conn, brokerage, "2025-01-01", 500.0, "USD");
        // Day 2: paid the credit card so cash drops to 600, but investments climb to 950.
        // Net worth still ticks up to 1550 — the exact "anxiety buffer" reassurance case.
        add_snapshot(&conn, chequing, "2025-02-01", 600.0, "USD");
        add_snapshot(&conn, brokerage, "2025-02-01", 950.0, "USD");

        let d = compute_net_worth_delta(&conn).unwrap();
        assert!(d.has_previous);
        assert_eq!(d.current_date.as_deref(), Some("2025-02-01"));
        assert_eq!(d.previous_date.as_deref(), Some("2025-01-01"));

        // Current split.
        assert_eq!(d.total.usd, 1550.0);
        assert_eq!(d.liquid.usd, 600.0);
        assert_eq!(d.invested.usd, 950.0);

        // Cash fell 400 but net worth rose 50 (investments +450).
        assert_eq!(d.liquid_delta.usd, -400.0);
        assert_eq!(d.invested_delta.usd, 450.0);
        assert_eq!(d.total_delta.usd, 50.0);

        // CAD mirror is exactly double at this rate.
        assert_eq!(d.liquid_delta.cad, -800.0);
        assert_eq!(d.total_delta.cad, 100.0);
    }

    #[test]
    fn delta_excludes_accounts_added_after_the_previous_date() {
        let conn = setup();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2025-01-01')",
            [],
        )
        .unwrap();

        let chequing = add_typed_account(&conn, "Chase Checking", "USD", "chequing");
        let brokerage = add_typed_account(&conn, "Robinhood", "USD", "brokerage");

        // The chequing account exists on both dates: a real +100 move.
        add_snapshot(&conn, chequing, "2025-01-01", 1000.0, "USD");
        add_snapshot(&conn, chequing, "2025-02-01", 1100.0, "USD");
        // The brokerage's first-ever snapshot lands on the latest date — it's coming online,
        // not money earned, so it must not inflate the delta.
        add_snapshot(&conn, brokerage, "2025-02-01", 5000.0, "USD");

        let d = compute_net_worth_delta(&conn).unwrap();
        assert!(d.has_previous);
        assert_eq!(d.current_date.as_deref(), Some("2025-02-01"));
        assert_eq!(d.previous_date.as_deref(), Some("2025-01-01"));

        // Current totals still reflect *all* accounts, including the brand-new brokerage.
        assert_eq!(d.total.usd, 6100.0);
        assert_eq!(d.liquid.usd, 1100.0);
        assert_eq!(d.invested.usd, 5000.0);

        // The delta is like-for-like: only the chequing account existed on both dates, so net
        // worth is up 100 — not 5100. The new brokerage contributes nothing to the delta.
        assert_eq!(d.total_delta.usd, 100.0);
        assert_eq!(d.liquid_delta.usd, 100.0);
        assert_eq!(d.invested_delta.usd, 0.0);
        assert_eq!(d.total_delta.cad, 200.0);

        let period = compute_net_worth_period(&conn, 30).unwrap().unwrap();
        assert_eq!(period.since, "2025-01-01");
        assert_eq!(period.through, "2025-02-01");
        assert_eq!(period.window_days, 31);
        assert_eq!(period.account_ids.len(), 1);
        assert!(period.account_ids.contains(&chequing));
        assert_eq!(period.change.usd, 100.0);
    }

    #[test]
    fn delta_has_no_previous_with_a_single_date() {
        let conn = setup();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 2.0, '2025-01-01')",
            [],
        )
        .unwrap();

        let savings = add_typed_account(&conn, "Bask Savings", "USD", "savings");
        add_snapshot(&conn, savings, "2025-01-01", 100.0, "USD");

        let d = compute_net_worth_delta(&conn).unwrap();
        assert!(!d.has_previous);
        assert_eq!(d.liquid.usd, 100.0);
        // With nothing to compare against, deltas are zero rather than the full balance.
        assert_eq!(d.total_delta, MoneyPair::default());
        assert_eq!(d.liquid_delta, MoneyPair::default());
    }

    #[test]
    fn delta_is_empty_without_any_snapshots() {
        let conn = setup();
        add_typed_account(&conn, "Empty", "USD", "chequing");
        let d = compute_net_worth_delta(&conn).unwrap();
        assert!(!d.has_previous);
        assert_eq!(d.current_date, None);
        assert_eq!(d.total, MoneyPair::default());
    }
}
