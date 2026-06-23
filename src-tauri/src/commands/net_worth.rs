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

/// How an account contributes to the "Anxiety Buffer" split. `Liquid` is spendable cash
/// (chequing/savings) — the balance that drops after paying a credit card and triggers panic.
/// `Invested` is the long-horizon pile that usually offsets it. Liabilities (credit) and anything
/// else still count toward the net-worth total but aren't broken out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum AccountClass {
    Liquid,
    Invested,
    Other,
}

fn account_class(account_type: &str) -> AccountClass {
    match account_type {
        "chequing" | "savings" => AccountClass::Liquid,
        "brokerage" | "tfsa" | "rrsp" | "fhsa" | "401k" | "ira" | "roth_ira" | "crypto" => {
            AccountClass::Invested
        }
        _ => AccountClass::Other,
    }
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

    for (id, name, institution, account_type, jurisdiction, currency, balance_opt, snapshot_date) in
        account_rows
    {
        let balance = balance_opt.unwrap_or(0.0);

        let (balance_usd, balance_cad) = convert_balance(balance, &currency, &usd_rates);
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
/// For each date on which any account has a balance snapshot, every account's most
/// recent balance *as of that date* is carried forward (accounts contribute 0 before
/// their first snapshot), converted with the latest stored FX rate, and summed. Using
/// a single consistent FX rate keeps the trend driven by balances, not FX noise.
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
    let mut stmt =
        conn.prepare("SELECT id, currency, account_type FROM accounts WHERE is_active = 1")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, currency, account_type) = row?;
        meta.insert(id, (currency, account_class(&account_type)));
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
            AccountClass::Other => {}
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

// ---------------------------------------------------------------------------
// Backfill — reconstruct historical balance snapshots from transactions
// ---------------------------------------------------------------------------

/// Snapshots written by the backfill carry this `source` so they can be safely deleted and
/// recomputed without touching real snapshots (`manual` / `simplefin` / `import` / `snaptrade`
/// / `questrade`).
const BACKFILL_SOURCE: &str = "backfill";

/// Summary of a backfill run, surfaced to the UI so it can reload the chart and explain the result.
#[derive(Debug, Serialize, PartialEq, Default)]
pub struct BackfillResult {
    /// Accounts that had an anchor snapshot and at least one earlier dated transaction.
    pub accounts_backfilled: usize,
    /// Total reconstructed snapshots inserted (excludes dates that already had a real snapshot).
    pub snapshots_created: usize,
    /// Distinct snapshot dates present after the backfill (real + reconstructed). The chart needs
    /// at least two to draw a line.
    pub distinct_dates: usize,
    /// Oldest reconstructed date, or None when nothing was created.
    pub earliest_date: Option<String>,
}

/// Reconstruct a net-worth history from existing transactions for accounts that only have a single
/// (or sparse) balance snapshot, so the chart has something to draw before daily sync history
/// accumulates.
///
/// For each active account we take its most recent ("anchor") snapshot — balance `A` on date `D0` —
/// and roll the balance backward. Because a positive transaction amount raises the balance and a
/// negative one lowers it, the balance at the end of an earlier day `d` is:
///
/// ```text
/// balance(d) = A - sum(amount of txns with d < txn_date <= D0)
/// ```
///
/// We evaluate this for every account on a *shared* date grid (the union of all transaction dates),
/// not just the dates an individual account transacted on. That way an account with no recorded
/// activity between two dates is carried flat across them — its balance is assumed unchanged —
/// instead of dropping to zero and then "popping" back in, which would put misleading cliffs in the
/// total line.
///
/// Reconstructed rows are written with `source = "backfill"`. The run first deletes any prior
/// backfill rows (so it is idempotent and refreshes when transactions or the anchor change) and
/// inserts with `INSERT OR IGNORE`, so a real snapshot on the same date always wins.
///
/// Caveat: this only backs out recorded cash flows. For investment / brokerage accounts the balance
/// also moves with the market, so reconstructed history for those accounts is approximate.
#[tauri::command]
pub fn backfill_net_worth_history(db: State<AppDb>) -> Result<BackfillResult, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    backfill_history(&mut conn).map_err(|e| e.to_string())
}

pub(crate) fn backfill_history(conn: &mut Connection) -> rusqlite::Result<BackfillResult> {
    let tx = conn.transaction()?;

    // Replace prior backfill rows so re-runs refresh rather than duplicate or go stale.
    tx.execute(
        "DELETE FROM balance_snapshots WHERE source = ?1",
        [BACKFILL_SOURCE],
    )?;

    // The shared date grid: every distinct transaction date across active accounts, newest first.
    // Each account is reconstructed on this grid so all accounts span the same range.
    let grid: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT t.txn_date FROM transactions t \
             JOIN accounts a ON a.id = t.account_id \
             WHERE a.is_active = 1 ORDER BY t.txn_date DESC",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    // Active accounts and the currency to stamp reconstructed snapshots with.
    let accounts: Vec<(i64, String)> = {
        let mut stmt = tx.prepare("SELECT id, currency FROM accounts WHERE is_active = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let mut accounts_backfilled = 0usize;
    let mut snapshots_created = 0usize;
    let mut earliest_date: Option<String> = None;

    for (account_id, account_currency) in accounts {
        // Anchor on the most recent snapshot: balance `A` on date `D0`.
        let anchor: Option<(String, f64, Option<String>)> = tx
            .query_row(
                "SELECT snapshot_date, balance, currency FROM balance_snapshots \
                 WHERE account_id = ?1 ORDER BY snapshot_date DESC LIMIT 1",
                [account_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .ok();
        let (anchor_date, anchor_balance, anchor_currency) = match anchor {
            Some(a) => a,
            None => continue, // nothing to anchor from
        };
        let snapshot_currency = anchor_currency.unwrap_or_else(|| account_currency.clone());

        // This account's net flow per date, up to and including the anchor date. Transactions dated
        // after the anchor are ignored: we never reconstruct forward past the balance we know.
        let delta_by_date: HashMap<String, f64> = {
            let mut stmt = tx.prepare(
                "SELECT txn_date, SUM(amount) FROM transactions \
                 WHERE account_id = ?1 AND txn_date <= ?2 GROUP BY txn_date",
            )?;
            let rows = stmt.query_map(rusqlite::params![account_id, anchor_date], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?;
            rows.collect::<Result<HashMap<_, _>, _>>()?
        };

        // Walk the shared grid newest -> oldest. `subtract` is the sum of this account's amounts
        // strictly after the current grid date; seed it with anything dated on the anchor itself
        // (already baked into `A`, so it must come back out for every earlier date).
        let mut subtract = *delta_by_date.get(&anchor_date).unwrap_or(&0.0);
        let mut created_for_account = false;
        for date in &grid {
            if *date >= anchor_date {
                continue; // on/after the anchor: nothing to reconstruct
            }
            let balance = anchor_balance - subtract;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO balance_snapshots \
                 (account_id, snapshot_date, balance, currency, source) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![account_id, date, balance, snapshot_currency, BACKFILL_SOURCE],
            )?;
            if inserted > 0 {
                snapshots_created += 1;
                created_for_account = true;
                earliest_date = Some(match earliest_date {
                    Some(cur) if cur <= *date => cur,
                    _ => date.clone(),
                });
            }
            subtract += *delta_by_date.get(date).unwrap_or(&0.0);
        }
        if created_for_account {
            accounts_backfilled += 1;
        }
    }

    let distinct_dates: usize = tx.query_row(
        "SELECT COUNT(DISTINCT bs.snapshot_date) FROM balance_snapshots bs \
         JOIN accounts a ON a.id = bs.account_id WHERE a.is_active = 1",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize;

    tx.commit()?;

    Ok(BackfillResult {
        accounts_backfilled,
        snapshots_created,
        distinct_dates,
        earliest_date,
    })
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

    fn add_txn(conn: &Connection, account_id: i64, date: &str, amount: f64, currency: &str) {
        conn.execute(
            "INSERT INTO transactions (account_id, txn_date, description, amount, currency) \
             VALUES (?1, ?2, 'txn', ?3, ?4)",
            rusqlite::params![account_id, date, amount, currency],
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

    #[test]
    fn backfill_reconstructs_history_from_transactions() {
        let mut conn = setup();
        let acct = add_typed_account(&conn, "Chase Checking", "USD", "chequing");

        // Anchor: balance 1000 on 2025-03-01. Positive amounts raised the balance, negatives lowered it.
        add_snapshot(&conn, acct, "2025-03-01", 1000.0, "USD");
        add_txn(&conn, acct, "2025-01-15", 200.0, "USD"); // deposit
        add_txn(&conn, acct, "2025-02-10", -50.0, "USD"); // spend
        add_txn(&conn, acct, "2025-03-01", 100.0, "USD"); // on the anchor date (already in 1000)

        let res = backfill_history(&mut conn).unwrap();
        assert_eq!(res.accounts_backfilled, 1);
        assert_eq!(res.snapshots_created, 2);
        assert_eq!(res.distinct_dates, 3);
        assert_eq!(res.earliest_date.as_deref(), Some("2025-01-15"));

        // balance(2025-02-10) = 1000 - 100 (the later +100) = 900.
        // balance(2025-01-15) = 1000 - (100 - 50) = 950.
        let series = compute_net_worth_history(&conn).unwrap();
        assert_eq!(series.len(), 3);
        assert_eq!(series[0].date, "2025-01-15");
        assert_eq!(series[0].total_usd, 950.0);
        assert_eq!(series[1].date, "2025-02-10");
        assert_eq!(series[1].total_usd, 900.0);
        assert_eq!(series[2].date, "2025-03-01");
        assert_eq!(series[2].total_usd, 1000.0);
    }

    #[test]
    fn backfill_does_not_overwrite_real_snapshots() {
        let mut conn = setup();
        let acct = add_typed_account(&conn, "Chase Checking", "USD", "chequing");

        add_snapshot(&conn, acct, "2025-03-01", 1000.0, "USD");
        // A real manual snapshot already exists on an intermediate date.
        add_snapshot(&conn, acct, "2025-02-10", 777.0, "USD");
        add_txn(&conn, acct, "2025-01-15", 200.0, "USD");
        add_txn(&conn, acct, "2025-02-10", -50.0, "USD");

        let res = backfill_history(&mut conn).unwrap();
        // Only 2025-01-15 is newly created; 2025-02-10 is left untouched.
        assert_eq!(res.snapshots_created, 1);

        let (balance, source): (f64, String) = conn
            .query_row(
                "SELECT balance, source FROM balance_snapshots \
                 WHERE account_id = ?1 AND snapshot_date = '2025-02-10'",
                [acct],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(balance, 777.0);
        assert_eq!(source, "manual");
    }

    #[test]
    fn backfill_is_idempotent() {
        let mut conn = setup();
        let acct = add_typed_account(&conn, "Chase Checking", "USD", "chequing");
        add_snapshot(&conn, acct, "2025-03-01", 1000.0, "USD");
        add_txn(&conn, acct, "2025-01-15", 200.0, "USD");
        add_txn(&conn, acct, "2025-02-10", -50.0, "USD");

        let first = backfill_history(&mut conn).unwrap();
        let second = backfill_history(&mut conn).unwrap();
        assert_eq!(first, second);

        // Re-running deletes prior backfill rows first, so there are no duplicates.
        let backfill_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots WHERE source = 'backfill'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(backfill_rows, 2);
        assert_eq!(compute_net_worth_history(&conn).unwrap().len(), 3);
    }

    #[test]
    fn backfill_skips_account_without_anchor() {
        let mut conn = setup();
        let acct = add_typed_account(&conn, "No Snapshot", "USD", "chequing");
        add_txn(&conn, acct, "2025-01-15", 200.0, "USD");

        let res = backfill_history(&mut conn).unwrap();
        assert_eq!(res.accounts_backfilled, 0);
        assert_eq!(res.snapshots_created, 0);
        assert_eq!(res.distinct_dates, 0);
        assert!(compute_net_worth_history(&conn).unwrap().is_empty());
    }

    #[test]
    fn backfill_excludes_transactions_after_anchor() {
        let mut conn = setup();
        let acct = add_typed_account(&conn, "Chase Checking", "USD", "chequing");

        add_snapshot(&conn, acct, "2025-02-01", 500.0, "USD");
        add_txn(&conn, acct, "2025-01-15", 100.0, "USD");
        // A transaction dated after the anchor must not affect the reconstruction.
        add_txn(&conn, acct, "2025-03-01", 1000.0, "USD");

        let res = backfill_history(&mut conn).unwrap();
        assert_eq!(res.snapshots_created, 1);

        // balance(2025-01-15) = 500 (the post-anchor +1000 is ignored, and nothing falls between
        // 2025-01-15 and the 2025-02-01 anchor).
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots \
                 WHERE account_id = ?1 AND snapshot_date = '2025-01-15'",
                [acct],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 500.0);

        // No snapshot was invented on the post-anchor transaction's date.
        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots \
                 WHERE account_id = ?1 AND snapshot_date = '2025-03-01'",
                [acct],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0);
    }

    #[test]
    fn backfill_carries_accounts_across_the_shared_grid() {
        // Two accounts share an anchor date. Account A transacts on two earlier dates; account B has
        // no transactions at all. B must be carried flat across A's dates (present from the start at
        // its anchor balance) rather than contributing 0 until the anchor and then popping in.
        let mut conn = setup();
        let a = add_typed_account(&conn, "Chase Checking", "USD", "chequing");
        let b = add_typed_account(&conn, "Bask Savings", "USD", "savings");

        add_snapshot(&conn, a, "2025-03-01", 1000.0, "USD");
        add_snapshot(&conn, b, "2025-03-01", 500.0, "USD");
        add_txn(&conn, a, "2025-01-10", -100.0, "USD");
        add_txn(&conn, a, "2025-02-01", 200.0, "USD");

        let res = backfill_history(&mut conn).unwrap();
        // A: 2025-01-10 + 2025-02-01; B carried onto both of A's dates -> 4 rows, 2 accounts.
        assert_eq!(res.accounts_backfilled, 2);
        assert_eq!(res.snapshots_created, 4);

        // B is reconstructed flat at 500 on a date it never transacted on.
        let b_early: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots \
                 WHERE account_id = ?1 AND snapshot_date = '2025-01-10'",
                [b],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b_early, 500.0);

        // The total line has no cliffs: A 800 + B 500, then A 1000 + B 500, then the anchors.
        let series = compute_net_worth_history(&conn).unwrap();
        assert_eq!(series.len(), 3);
        assert_eq!(series[0].date, "2025-01-10");
        assert_eq!(series[0].total_usd, 1300.0);
        assert_eq!(series[1].date, "2025-02-01");
        assert_eq!(series[1].total_usd, 1500.0);
        assert_eq!(series[2].date, "2025-03-01");
        assert_eq!(series[2].total_usd, 1500.0);
    }
}
