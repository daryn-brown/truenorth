//! Cashflow + fixed/variable tagging.
//!
//! Synced transactions are classified into one of four flows and summed over a rolling window to
//! produce a savings-rate view that separates *fixed* commitments (the $800/mo support to mom)
//! from *variable* lifestyle spending, and excludes internal *transfers* (credit-card payments,
//! account-to-account moves) so nothing double-counts.
//!
//! Classification precedence, highest first:
//! 1. a per-transaction manual override (`transactions.flow_override`);
//! 2. the first matching rule in `txn_rules` (case-insensitive substring of the description,
//!    earlier rows win);
//! 3. a sign default — money in is income, money out is variable spending.
//!
//! Every figure is carried in both USD and CAD via the same USD-pivot FX map net worth uses, so
//! the frontend can render either side of the currency toggle without a second round-trip.

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::State;

use super::net_worth::{convert_balance, MoneyPair};
use crate::db::AppDb;
use crate::fx::load_usd_rates;

/// How a transaction is counted toward cashflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowType {
    /// Money in (salary, RSUs, interest, refunds).
    Income,
    /// A recurring committed expense — rent, the monthly support to mom. Not lifestyle creep.
    Fixed,
    /// Discretionary spending. This is the "lifestyle creep" number to watch.
    Variable,
    /// An internal move (card payment, account-to-account). Excluded from income and expenses.
    Transfer,
}

impl FlowType {
    fn as_str(self) -> &'static str {
        match self {
            FlowType::Income => "income",
            FlowType::Fixed => "fixed",
            FlowType::Variable => "variable",
            FlowType::Transfer => "transfer",
        }
    }

    /// Parse a stored/user-supplied flow type, accepting any casing. Unknown values yield `None`.
    fn parse(s: &str) -> Option<FlowType> {
        match s.trim().to_ascii_lowercase().as_str() {
            "income" => Some(FlowType::Income),
            "fixed" => Some(FlowType::Fixed),
            "variable" => Some(FlowType::Variable),
            "transfer" => Some(FlowType::Transfer),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

/// A classification rule: a case-insensitive substring matched against a transaction description.
#[derive(Debug, Clone, Serialize)]
pub struct TxnRule {
    pub id: i64,
    pub pattern: String,
    pub flow_type: String,
}

/// One transaction with its resolved flow type, for the recent-transactions list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClassifiedTransaction {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub txn_date: String,
    pub description: String,
    pub amount: f64,
    pub currency: String,
    pub flow_type: String,
    /// True when the flow type comes from a manual override rather than a rule/sign default.
    pub is_override: bool,
}

/// Rolling-window cashflow totals, with each figure in both reporting currencies.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CashflowSummary {
    /// Length of the window in days.
    pub window_days: i64,
    /// Inclusive start date of the window (YYYY-MM-DD).
    pub since: String,
    pub income: MoneyPair,
    /// Fixed expenses as a positive magnitude.
    pub fixed: MoneyPair,
    /// Variable ("lifestyle") expenses as a positive magnitude.
    pub variable: MoneyPair,
    /// income − fixed − variable. Negative means spending outran income over the window.
    pub net_savings: MoneyPair,
    /// net_savings / income (USD basis), 0 when there was no income.
    pub savings_rate: f64,
    /// Count of transfer rows excluded from the totals.
    pub transfer_count: i64,
    /// Total transactions considered in the window.
    pub txn_count: i64,
    /// True when at least one transaction's currency had no stored FX rate (so it counted as 0).
    pub currency_warning: bool,
}

// ---------------------------------------------------------------------------
// Classification (pure)
// ---------------------------------------------------------------------------

/// Resolve a transaction's flow type. `override_opt` is the stored manual override (if any);
/// `rules` are `(lowercased pattern, flow type)` pairs in priority order.
fn classify(
    description: &str,
    amount: f64,
    override_opt: Option<&str>,
    rules: &[(String, FlowType)],
) -> FlowType {
    if let Some(ft) = override_opt.and_then(FlowType::parse) {
        return ft;
    }
    let hay = description.to_ascii_lowercase();
    for (pattern, ft) in rules {
        if !pattern.is_empty() && hay.contains(pattern.as_str()) {
            return *ft;
        }
    }
    if amount >= 0.0 {
        FlowType::Income
    } else {
        FlowType::Variable
    }
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

/// Load classification rules in priority order, patterns lowercased for matching.
fn load_rules(conn: &Connection) -> rusqlite::Result<Vec<(String, FlowType)>> {
    let mut stmt = conn.prepare("SELECT pattern, flow_type FROM txn_rules ORDER BY id ASC")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut rules = Vec::new();
    for row in rows {
        let (pattern, flow_type) = row?;
        if let Some(ft) = FlowType::parse(&flow_type) {
            rules.push((pattern.to_ascii_lowercase(), ft));
        }
    }
    Ok(rules)
}

/// ISO date `window_days` ago (inclusive lower bound of the rolling window).
fn window_since(window_days: i64) -> String {
    let days = window_days.clamp(1, 3650);
    (chrono::Utc::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

/// Compute the cashflow summary over the trailing `window_days` across active accounts.
fn compute_cashflow(conn: &Connection, window_days: i64) -> rusqlite::Result<CashflowSummary> {
    let window_days = window_days.clamp(1, 3650);
    let since = window_since(window_days);
    let usd_rates = load_usd_rates(conn)?;
    let rules = load_rules(conn)?;

    let mut stmt = conn.prepare(
        "SELECT t.amount, t.currency, t.description, t.flow_override \
         FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE a.is_active = 1 AND t.txn_date >= ?1",
    )?;
    let rows = stmt.query_map(params![since], |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut income = MoneyPair::default();
    let mut fixed = MoneyPair::default();
    let mut variable = MoneyPair::default();
    let mut transfer_count = 0i64;
    let mut txn_count = 0i64;
    let mut currency_warning = false;

    for row in rows {
        let (amount, currency, description, override_opt) = row?;
        txn_count += 1;
        if currency != "USD" && !usd_rates.contains_key(&currency) {
            currency_warning = true;
        }
        let ft = classify(&description, amount, override_opt.as_deref(), &rules);
        let (usd, cad) = convert_balance(amount, &currency, &usd_rates);
        match ft {
            FlowType::Income => {
                income.usd += usd;
                income.cad += cad;
            }
            // Expenses arrive as negative amounts; store them as positive magnitudes.
            FlowType::Fixed => {
                fixed.usd -= usd;
                fixed.cad -= cad;
            }
            FlowType::Variable => {
                variable.usd -= usd;
                variable.cad -= cad;
            }
            FlowType::Transfer => transfer_count += 1,
        }
    }

    let net_savings = MoneyPair {
        usd: income.usd - fixed.usd - variable.usd,
        cad: income.cad - fixed.cad - variable.cad,
    };
    let savings_rate = if income.usd > 0.0 {
        net_savings.usd / income.usd
    } else {
        0.0
    };

    Ok(CashflowSummary {
        window_days,
        since,
        income,
        fixed,
        variable,
        net_savings,
        savings_rate,
        transfer_count,
        txn_count,
        currency_warning,
    })
}

/// Most recent transactions (active accounts) with their resolved flow type.
fn fetch_classified(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<ClassifiedTransaction>> {
    let limit = limit.clamp(1, 1000);
    let rules = load_rules(conn)?;

    let mut stmt = conn.prepare(
        "SELECT t.id, t.account_id, a.name, t.txn_date, t.description, t.amount, t.currency, \
                t.flow_override \
         FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE a.is_active = 1 \
         ORDER BY t.txn_date DESC, t.id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, f64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, Option<String>>(7)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, account_id, account_name, txn_date, description, amount, currency, override_opt) =
            row?;
        let is_override = override_opt.as_deref().and_then(FlowType::parse).is_some();
        let ft = classify(&description, amount, override_opt.as_deref(), &rules);
        out.push(ClassifiedTransaction {
            id,
            account_id,
            account_name,
            txn_date,
            description,
            amount,
            currency,
            flow_type: ft.as_str().to_string(),
            is_override,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Income vs. fixed vs. variable spending (and the resulting savings rate) over the trailing
/// `window_days` (default 30).
#[tauri::command]
pub fn get_cashflow_summary(
    db: State<AppDb>,
    window_days: Option<i64>,
) -> Result<CashflowSummary, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    compute_cashflow(&conn, window_days.unwrap_or(30)).map_err(|e| e.to_string())
}

/// The most recent transactions with their resolved flow type, for review and retagging.
#[tauri::command]
pub fn list_recent_transactions(
    db: State<AppDb>,
    limit: Option<i64>,
) -> Result<Vec<ClassifiedTransaction>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    fetch_classified(&conn, limit.unwrap_or(50)).map_err(|e| e.to_string())
}

/// Set or clear a transaction's manual flow override. Passing `None` (or an empty string) reverts
/// it to automatic rule/sign classification.
#[tauri::command]
pub fn set_transaction_flow(
    db: State<AppDb>,
    transaction_id: i64,
    flow_type: Option<String>,
) -> Result<(), String> {
    let normalized = match flow_type.as_deref() {
        None | Some("") => None,
        Some(s) => Some(
            FlowType::parse(s)
                .ok_or_else(|| format!("Unknown flow type: {s}"))?
                .as_str(),
        ),
    };
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let changed = conn
        .execute(
            "UPDATE transactions SET flow_override = ?1 WHERE id = ?2",
            params![normalized, transaction_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("No transaction with id {transaction_id}"));
    }
    Ok(())
}

/// List the classification rules in priority order.
#[tauri::command]
pub fn list_txn_rules(db: State<AppDb>) -> Result<Vec<TxnRule>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, pattern, flow_type FROM txn_rules ORDER BY id ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TxnRule {
                id: r.get(0)?,
                pattern: r.get(1)?,
                flow_type: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Add a classification rule. New rules sort after existing ones (lower priority).
#[tauri::command]
pub fn add_txn_rule(
    db: State<AppDb>,
    pattern: String,
    flow_type: String,
) -> Result<TxnRule, String> {
    let pattern = pattern.trim().to_string();
    if pattern.is_empty() {
        return Err("Rule pattern can't be empty.".into());
    }
    let ft =
        FlowType::parse(&flow_type).ok_or_else(|| format!("Unknown flow type: {flow_type}"))?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO txn_rules (pattern, flow_type) VALUES (?1, ?2)",
        params![pattern, ft.as_str()],
    )
    .map_err(|e| e.to_string())?;
    Ok(TxnRule {
        id: conn.last_insert_rowid(),
        pattern,
        flow_type: ft.as_str().to_string(),
    })
}

/// Delete a classification rule by id.
#[tauri::command]
pub fn delete_txn_rule(db: State<AppDb>, rule_id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM txn_rules WHERE id = ?1", params![rule_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<(String, FlowType)> {
        vec![
            ("mom".into(), FlowType::Fixed),
            ("e-transfer".into(), FlowType::Transfer),
        ]
    }

    #[test]
    fn override_beats_rule_and_sign() {
        // "INTERAC E-TRANSFER TO MOM" would hit the mom rule, but an explicit override wins.
        assert_eq!(
            classify(
                "Interac e-transfer to Mom",
                -800.0,
                Some("variable"),
                &rules()
            ),
            FlowType::Variable
        );
    }

    #[test]
    fn rule_precedence_is_first_match() {
        // "mom" precedes "e-transfer", so the support payment is fixed, not an excluded transfer.
        assert_eq!(
            classify("INTERAC E-TRANSFER TO MOM", -800.0, None, &rules()),
            FlowType::Fixed
        );
        assert_eq!(
            classify("E-TRANSFER TO LANDLORD", -1500.0, None, &rules()),
            FlowType::Transfer
        );
    }

    #[test]
    fn sign_default_when_no_rule_matches() {
        assert_eq!(
            classify("STARBUCKS", -6.25, None, &rules()),
            FlowType::Variable
        );
        assert_eq!(
            classify("PAYROLL", 2500.0, None, &rules()),
            FlowType::Income
        );
    }

    fn seed_account(conn: &Connection, currency: &str) -> i64 {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Chequing', 'Scotiabank', 'chequing', ?1, 'CA')",
            params![currency],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_txn(
        conn: &Connection,
        account_id: i64,
        date: &str,
        description: &str,
        amount: f64,
        currency: &str,
    ) {
        conn.execute(
            "INSERT INTO transactions (account_id, txn_date, description, amount, currency, connector_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account_id, date, description, amount, currency, description],
        )
        .unwrap();
    }

    #[test]
    fn compute_cashflow_splits_fixed_variable_and_excludes_transfers() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        // 1 USD = 1.25 CAD.
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();

        let acct = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        insert_txn(&conn, acct, &today, "MICROSOFT PAYROLL", 5000.0, "CAD");
        insert_txn(
            &conn,
            acct,
            &today,
            "Interac e-transfer to MOM",
            -800.0,
            "CAD",
        );
        insert_txn(&conn, acct, &today, "GROCERY STORE", -200.0, "CAD");
        // Credit-card payment matches the seeded "payment - thank you" transfer rule → excluded.
        insert_txn(&conn, acct, &today, "PAYMENT - THANK YOU", -1000.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.txn_count, 4);
        assert_eq!(s.transfer_count, 1);
        assert_eq!(s.income.cad, 5000.0);
        assert_eq!(s.fixed.cad, 800.0); // mom support, flagged fixed
        assert_eq!(s.variable.cad, 200.0); // groceries only — the card payment didn't count
        assert_eq!(s.net_savings.cad, 4000.0);
        // USD figures use the 1.25 pivot.
        assert_eq!(s.income.usd, 4000.0);
        assert!((s.savings_rate - 0.8).abs() < 1e-9);
        assert!(!s.currency_warning);
    }

    #[test]
    fn compute_cashflow_flags_unconvertible_currency() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        let acct = seed_account(&conn, "JMD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        insert_txn(&conn, acct, &today, "SHOP", -500.0, "JMD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert!(s.currency_warning);
        // No USD rate for JMD, so it contributes 0 rather than a wrong number.
        assert_eq!(s.variable.usd, 0.0);
    }

    #[test]
    fn fetch_classified_reports_override_flag() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        let acct = seed_account(&conn, "CAD");
        insert_txn(&conn, acct, "2025-01-02", "GROCERY STORE", -50.0, "CAD");
        conn.execute(
            "UPDATE transactions SET flow_override = 'fixed' WHERE description = 'GROCERY STORE'",
            [],
        )
        .unwrap();

        let list = fetch_classified(&conn, 50).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].flow_type, "fixed");
        assert!(list[0].is_override);
    }
}
