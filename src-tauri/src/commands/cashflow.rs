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
//! 3. an account-type rule — a positive amount landing in a non-cash account (a credit card, or a
//!    brokerage/retirement account) is an internal move (card payment, contribution, security buy,
//!    reinvested dividend), never earned income;
//! 4. a sign default — money in is income, money out is variable spending.
//!
//! Every figure is carried in both USD and CAD via the same USD-pivot FX map net worth uses, so
//! the frontend can render either side of the currency toggle without a second round-trip.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
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
    /// Best-guess spending category: a stored AI/manual value if present, otherwise a local
    /// merchant guess. `None` when nothing matched.
    pub category: Option<String>,
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
    /// Variable spending broken down by category, largest (USD) first. Lets the UI and the AI
    /// see where lifestyle money actually goes.
    pub variable_by_category: Vec<CategoryTotal>,
}

/// One spending category and its total magnitude over the window, in both reporting currencies.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CategoryTotal {
    pub category: String,
    pub amount: MoneyPair,
}

// ---------------------------------------------------------------------------
// Classification (pure)
// ---------------------------------------------------------------------------

/// Resolve a transaction's flow type from its description, sign and account type. `override_opt`
/// is the stored manual override (if any); `account_type` is the owning account's type;
/// `rules` are `(lowercased pattern, flow type)` pairs in priority order.
fn classify(
    description: &str,
    amount: f64,
    override_opt: Option<&str>,
    account_type: &str,
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
        // Money landing in a non-cash account is an internal move, not earned income: a payment
        // toward a credit card, or a contribution / funding deposit / security buy / reinvested
        // dividend in a brokerage or retirement account. Treat it as a transfer so it never
        // inflates income. Outflows from these accounts (card purchases) still count normally.
        if is_non_cash_account(account_type) {
            FlowType::Transfer
        } else {
            FlowType::Income
        }
    } else {
        FlowType::Variable
    }
}

/// Account types whose *inflows* are internal moves rather than earned income: credit cards (an
/// inflow pays the card down) and investment / retirement accounts (inflows are contributions,
/// funding deposits, security purchases or reinvested dividends). Cash accounts (chequing,
/// savings) are deliberately absent — their inflows can be real income such as salary or interest.
fn is_non_cash_account(account_type: &str) -> bool {
    matches!(
        account_type.trim().to_ascii_lowercase().as_str(),
        "credit"
            | "brokerage"
            | "investment"
            | "crypto"
            | "ira"
            | "roth_ira"
            | "rrsp"
            | "tfsa"
            | "resp"
            | "lira"
            | "rrif"
            | "rlif"
            | "401k"
            | "pension"
    )
}

// ---------------------------------------------------------------------------
// Internal-transfer detection
// ---------------------------------------------------------------------------

/// How many days apart the two legs of an internal transfer may post and still be matched.
const TRANSFER_PAIR_WINDOW_DAYS: i64 = 5;

/// Cross-currency transfer legs (e.g. a USD debit funding a CAD deposit) rarely convert to exactly
/// the same USD value: the bank takes an FX spread, and our stored daily rate differs from the
/// transaction-time rate. Allow the converted magnitudes to differ by up to this fraction.
const CROSS_CURRENCY_TOLERANCE: f64 = 0.03;

/// The fields needed to resolve a transaction's flow across the whole window. Transfer detection
/// has to look at every row at once, not one at a time.
struct FlowInput {
    account_id: i64,
    date: String,
    description: String,
    amount: f64,
    currency: String,
    override_opt: Option<String>,
    account_type: String,
    category: Option<String>,
}

/// Whether any rule matches this description (sign-independent).
fn rule_matches(description: &str, rules: &[(String, FlowType)]) -> bool {
    let hay = description.to_ascii_lowercase();
    rules
        .iter()
        .any(|(pattern, _)| !pattern.is_empty() && hay.contains(pattern.as_str()))
}

// ---------------------------------------------------------------------------
// Merchant categorisation (pure, local)
// ---------------------------------------------------------------------------
// A deterministic best-guess of *what a purchase was for*, derived from the merchant in the
// description. Used for display and to ground the optional AI "Refine" pass. Independent of
// FlowType: a rent payment is flow=Fixed and category=Housing. Anything unmatched is left
// uncategorised for the AI pass (or the user) to fill in.

/// The canonical spending categories. Both the local guesser and the AI refine pass are limited
/// to this set so labels stay consistent across the app.
pub const CATEGORIES: &[&str] = &[
    "Groceries",
    "Dining",
    "Coffee",
    "Transport",
    "Fuel",
    "Shopping",
    "Subscriptions",
    "Utilities",
    "Housing",
    "Travel",
    "Health",
    "Entertainment",
    "Fees",
    "Education",
    "Gifts",
    "Charity",
    "Cash",
    "Income",
    "Transfer",
    "Other",
];

/// Lowercased-substring -> category table, checked in order (first hit wins). Ordering matters:
/// narrower buckets (Coffee, Groceries) precede broader ones (Dining, Shopping) so a cafe doesn't
/// fall into the generic food bucket. Curated to common North-American merchants.
const MERCHANT_CATEGORIES: &[(&str, &str)] = &[
    // Coffee (before Dining)
    ("starbucks", "Coffee"),
    ("tim hortons", "Coffee"),
    ("second cup", "Coffee"),
    ("dunkin", "Coffee"),
    ("peet's", "Coffee"),
    ("blenz", "Coffee"),
    ("coffee", "Coffee"),
    ("cafe", "Coffee"),
    // Groceries (before Dining/Shopping)
    ("loblaw", "Groceries"),
    ("no frills", "Groceries"),
    ("sobeys", "Groceries"),
    ("safeway", "Groceries"),
    ("save-on-foods", "Groceries"),
    ("save on foods", "Groceries"),
    ("whole foods", "Groceries"),
    ("trader joe", "Groceries"),
    ("costco", "Groceries"),
    ("superstore", "Groceries"),
    ("freshco", "Groceries"),
    ("food basics", "Groceries"),
    ("longo", "Groceries"),
    ("kroger", "Groceries"),
    ("aldi", "Groceries"),
    ("publix", "Groceries"),
    ("metro", "Groceries"),
    ("grocery", "Groceries"),
    ("supermarket", "Groceries"),
    // Dining / food delivery
    ("mcdonald", "Dining"),
    ("restaurant", "Dining"),
    ("uber eats", "Dining"),
    ("ubereats", "Dining"),
    ("doordash", "Dining"),
    ("skip the dishes", "Dining"),
    ("skipthedishes", "Dining"),
    ("grubhub", "Dining"),
    ("a&w", "Dining"),
    ("burger king", "Dining"),
    ("wendy", "Dining"),
    ("subway", "Dining"),
    ("chipotle", "Dining"),
    ("pizza", "Dining"),
    ("sushi", "Dining"),
    ("kitchen", "Dining"),
    ("grill", "Dining"),
    ("bar & grill", "Dining"),
    ("tavern", "Dining"),
    ("brewery", "Dining"),
    // Fuel
    ("shell", "Fuel"),
    ("esso", "Fuel"),
    ("petro-canada", "Fuel"),
    ("petro canada", "Fuel"),
    ("chevron", "Fuel"),
    ("husky", "Fuel"),
    ("ultramar", "Fuel"),
    ("exxon", "Fuel"),
    ("gas bar", "Fuel"),
    ("gas station", "Fuel"),
    ("gasoline", "Fuel"),
    ("petrol", "Fuel"),
    ("fuel", "Fuel"),
    // Transport / transit / rideshare
    ("uber", "Transport"),
    ("lyft", "Transport"),
    ("transit", "Transport"),
    ("translink", "Transport"),
    ("presto", "Transport"),
    ("compass vancouver", "Transport"),
    ("parking", "Transport"),
    ("via rail", "Transport"),
    ("taxi", "Transport"),
    // Travel
    ("air canada", "Travel"),
    ("westjet", "Travel"),
    ("airlines", "Travel"),
    ("airline", "Travel"),
    ("expedia", "Travel"),
    ("airbnb", "Travel"),
    ("hotel", "Travel"),
    ("marriott", "Travel"),
    ("hilton", "Travel"),
    ("booking.com", "Travel"),
    // Subscriptions / streaming / software
    ("netflix", "Subscriptions"),
    ("spotify", "Subscriptions"),
    ("disney+", "Subscriptions"),
    ("disney plus", "Subscriptions"),
    ("amazon prime", "Subscriptions"),
    ("prime video", "Subscriptions"),
    ("youtube premium", "Subscriptions"),
    ("apple.com/bill", "Subscriptions"),
    ("icloud", "Subscriptions"),
    ("patreon", "Subscriptions"),
    ("openai", "Subscriptions"),
    ("github", "Subscriptions"),
    ("adobe", "Subscriptions"),
    ("audible", "Subscriptions"),
    ("crave", "Subscriptions"),
    // Utilities / telecom
    ("hydro", "Utilities"),
    ("fortis", "Utilities"),
    ("enbridge", "Utilities"),
    ("rogers", "Utilities"),
    ("telus", "Utilities"),
    ("bell mobility", "Utilities"),
    ("bell canada", "Utilities"),
    ("shaw", "Utilities"),
    ("fido", "Utilities"),
    ("koodo", "Utilities"),
    ("freedom mobile", "Utilities"),
    ("at&t", "Utilities"),
    ("verizon", "Utilities"),
    ("comcast", "Utilities"),
    ("internet", "Utilities"),
    ("wireless", "Utilities"),
    ("utility", "Utilities"),
    // Health / pharmacy / fitness
    ("shoppers drug mart", "Health"),
    ("pharmacy", "Health"),
    ("pharmaprix", "Health"),
    ("rexall", "Health"),
    ("walgreens", "Health"),
    ("dental", "Health"),
    ("dentist", "Health"),
    ("clinic", "Health"),
    ("goodlife", "Health"),
    ("fitness", "Health"),
    ("gym", "Health"),
    // Entertainment
    ("cineplex", "Entertainment"),
    ("cinema", "Entertainment"),
    ("steam games", "Entertainment"),
    ("steampowered", "Entertainment"),
    ("playstation", "Entertainment"),
    ("xbox", "Entertainment"),
    ("nintendo", "Entertainment"),
    ("ticketmaster", "Entertainment"),
    // Shopping / retail / online
    ("amazon", "Shopping"),
    ("amzn", "Shopping"),
    ("ebay", "Shopping"),
    ("etsy", "Shopping"),
    ("best buy", "Shopping"),
    ("apple store", "Shopping"),
    ("ikea", "Shopping"),
    ("winners", "Shopping"),
    ("canadian tire", "Shopping"),
    ("home depot", "Shopping"),
    ("lowe's", "Shopping"),
    ("dollarama", "Shopping"),
    ("indigo", "Shopping"),
    ("lululemon", "Shopping"),
    ("aliexpress", "Shopping"),
    ("shein", "Shopping"),
    // Education
    ("tuition", "Education"),
    ("university", "Education"),
    ("college", "Education"),
    ("coursera", "Education"),
    ("udemy", "Education"),
    // Charity
    ("donation", "Charity"),
    ("red cross", "Charity"),
    ("unicef", "Charity"),
    // Housing
    ("mortgage", "Housing"),
    ("property tax", "Housing"),
    ("strata", "Housing"),
    ("condo fee", "Housing"),
    ("rent", "Housing"),
    // Fees / banking
    ("interest charge", "Fees"),
    ("overdraft", "Fees"),
    ("service charge", "Fees"),
    ("monthly fee", "Fees"),
    ("atm fee", "Fees"),
    ("foreign transaction", "Fees"),
    // Cash / ATM
    ("atm withdrawal", "Cash"),
    ("cash withdrawal", "Cash"),
    ("abm withdrawal", "Cash"),
    // Income
    ("payroll", "Income"),
    ("salary", "Income"),
    ("direct deposit", "Income"),
    ("dividend", "Income"),
    ("refund", "Income"),
];

/// Best-guess spending category from a description, or `None` if nothing matches. Pure and
/// case-insensitive; used for display and to ground the AI refine pass.
pub fn category_for(description: &str) -> Option<&'static str> {
    let hay = description.to_ascii_lowercase();
    MERCHANT_CATEGORIES
        .iter()
        .find(|(needle, _)| hay.contains(needle))
        .map(|(_, cat)| *cat)
}

/// Map an arbitrary label onto a canonical category (case-insensitive), or `None` if it isn't one
/// of [`CATEGORIES`]. Used to sanitise AI-suggested labels before they're stored.
pub fn canonical_category(label: &str) -> Option<&'static str> {
    let trimmed = label.trim();
    CATEGORIES
        .iter()
        .copied()
        .find(|c| c.eq_ignore_ascii_case(trimmed))
}

/// Resolve the category shown for a transaction: a stored (AI/manual) category wins over the
/// local guess; blank stored values are ignored.
fn resolve_category(stored: Option<&str>, description: &str) -> Option<String> {
    if let Some(s) = stored {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    category_for(description).map(|s| s.to_string())
}

/// Amount in whole cents, for exact equality matching without float wobble.
fn cents(amount: f64) -> i64 {
    (amount * 100.0).round() as i64
}

/// Absolute day gap between two `YYYY-MM-DD` dates, or `None` if either fails to parse.
fn day_gap(a: &str, b: &str) -> Option<i64> {
    let da = chrono::NaiveDate::parse_from_str(a, "%Y-%m-%d").ok()?;
    let db = chrono::NaiveDate::parse_from_str(b, "%Y-%m-%d").ok()?;
    Some((da - db).num_days().abs())
}

/// Absolute value of a transaction converted to USD, or 0 when its currency has no rate.
fn usd_magnitude(amount: f64, currency: &str, usd_rates: &HashMap<String, f64>) -> f64 {
    convert_balance(amount, currency, usd_rates).0.abs()
}

/// Decide whether an outflow and an inflow look like the two legs of one internal transfer,
/// returning a closeness score (lower is tighter) when they do. Legs must sit in different accounts
/// and post within the pairing window. Same-currency legs must match to the cent (internal moves
/// are exact); cross-currency legs only need their USD-converted magnitudes to agree within
/// `CROSS_CURRENCY_TOLERANCE`, which is what lets US<->Canada account moves be detected.
fn transfer_match(
    out: &FlowInput,
    inflow: &FlowInput,
    usd_rates: &HashMap<String, f64>,
) -> Option<f64> {
    if out.account_id == inflow.account_id {
        return None;
    }
    match day_gap(&out.date, &inflow.date) {
        Some(gap) if gap <= TRANSFER_PAIR_WINDOW_DAYS => {}
        _ => return None,
    }
    if out.currency == inflow.currency {
        (cents(out.amount).abs() == cents(inflow.amount).abs()).then_some(0.0)
    } else {
        let mo = usd_magnitude(out.amount, &out.currency, usd_rates);
        let mi = usd_magnitude(inflow.amount, &inflow.currency, usd_rates);
        if mo <= 0.0 || mi <= 0.0 {
            return None;
        }
        let rel = (mo - mi).abs() / mo.max(mi);
        (rel <= CROSS_CURRENCY_TOLERANCE).then_some(rel)
    }
}

/// Resolve every transaction's flow type, additionally detecting internal transfers between the
/// user's own accounts — a debit in one account matched by a credit in another within a few days —
/// that no keyword rule caught. Pairs may be same-currency (matched to the cent) or cross-currency
/// (matched by USD-converted value), so moving money between US and Canadian accounts no longer
/// inflates income and spending. Manual overrides and rule matches always win; auto-matching only
/// reclassifies rows that would otherwise fall to the income/variable sign default (plus the
/// un-keyworded partner of a keyworded transfer leg).
fn resolve_flow_types(
    txns: &[FlowInput],
    rules: &[(String, FlowType)],
    usd_rates: &HashMap<String, f64>,
) -> Vec<FlowType> {
    let n = txns.len();
    let mut flows: Vec<FlowType> = txns
        .iter()
        .map(|t| {
            classify(
                &t.description,
                t.amount,
                t.override_opt.as_deref(),
                &t.account_type,
                rules,
            )
        })
        .collect();

    // A row is "pinned" when an override or rule decided it — auto-matching must not move it.
    let pinned: Vec<bool> = txns
        .iter()
        .map(|t| {
            t.override_opt.as_deref().and_then(FlowType::parse).is_some()
                || rule_matches(&t.description, rules)
        })
        .collect();

    // A row may join a transfer pair only if it isn't pinned to a non-transfer flow: either it's a
    // sign-default row, or it's already a transfer (so the keyworded leg of a pair can also pull an
    // un-keyworded partner out of income/spending).
    let eligible: Vec<bool> = (0..n)
        .map(|i| !pinned[i] || flows[i] == FlowType::Transfer)
        .collect();

    // Match the largest outflows first so a big transfer claims its true partner before a smaller
    // coincidental amount can.
    let mut outflows: Vec<usize> = (0..n)
        .filter(|&i| eligible[i] && txns[i].amount < 0.0)
        .collect();
    outflows.sort_by(|&a, &b| {
        let ma = usd_magnitude(txns[a].amount, &txns[a].currency, usd_rates);
        let mb = usd_magnitude(txns[b].amount, &txns[b].currency, usd_rates);
        mb.partial_cmp(&ma).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut used = vec![false; n];
    for i in outflows {
        if used[i] {
            continue;
        }
        // Among the eligible unclaimed inflows, take the closest match (then the nearest in time).
        let mut best: Option<(usize, f64, i64)> = None;
        for j in 0..n {
            if used[j] || j == i || !eligible[j] || txns[j].amount <= 0.0 {
                continue;
            }
            let Some(closeness) = transfer_match(&txns[i], &txns[j], usd_rates) else {
                continue;
            };
            let gap = day_gap(&txns[i].date, &txns[j].date).unwrap_or(i64::MAX);
            let improves = match best {
                None => true,
                Some((_, best_close, best_gap)) => {
                    closeness < best_close - f64::EPSILON
                        || ((closeness - best_close).abs() <= f64::EPSILON && gap < best_gap)
                }
            };
            if improves {
                best = Some((j, closeness, gap));
            }
        }
        if let Some((j, _, _)) = best {
            // Matched pair: exclude both legs. Pinned legs keep their flow (already transfer);
            // sign-default legs flip to transfer so the inflow no longer reads as income.
            if !pinned[i] {
                flows[i] = FlowType::Transfer;
            }
            if !pinned[j] {
                flows[j] = FlowType::Transfer;
            }
            used[i] = true;
            used[j] = true;
        }
    }

    flows
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
pub(crate) fn compute_cashflow(conn: &Connection, window_days: i64) -> rusqlite::Result<CashflowSummary> {
    let window_days = window_days.clamp(1, 3650);
    let since = window_since(window_days);
    let usd_rates = load_usd_rates(conn)?;
    let rules = load_rules(conn)?;

    let mut stmt = conn.prepare(
        "SELECT t.account_id, t.txn_date, t.amount, t.currency, t.description, t.flow_override, \
                a.account_type, t.category \
         FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE a.is_active = 1 AND t.txn_date >= ?1",
    )?;
    let txns: Vec<FlowInput> = stmt
        .query_map(params![since], |r| {
            Ok(FlowInput {
                account_id: r.get(0)?,
                date: r.get(1)?,
                amount: r.get(2)?,
                currency: r.get(3)?,
                description: r.get(4)?,
                override_opt: r.get(5)?,
                account_type: r.get(6)?,
                category: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let flows = resolve_flow_types(&txns, &rules, &usd_rates);

    let mut income = MoneyPair::default();
    let mut fixed = MoneyPair::default();
    let mut variable = MoneyPair::default();
    let mut by_category: HashMap<String, MoneyPair> = HashMap::new();
    let mut transfer_count = 0i64;
    let txn_count = txns.len() as i64;
    let mut currency_warning = false;

    for (t, ft) in txns.iter().zip(flows.iter()) {
        if t.currency != "USD" && !usd_rates.contains_key(&t.currency) {
            currency_warning = true;
        }
        let (usd, cad) = convert_balance(t.amount, &t.currency, &usd_rates);
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
                let cat = resolve_category(t.category.as_deref(), &t.description)
                    .unwrap_or_else(|| "Uncategorized".to_string());
                let entry = by_category.entry(cat).or_default();
                entry.usd -= usd;
                entry.cad -= cad;
            }
            FlowType::Transfer => transfer_count += 1,
        }
    }

    let mut variable_by_category: Vec<CategoryTotal> = by_category
        .into_iter()
        .map(|(category, amount)| CategoryTotal { category, amount })
        .collect();
    variable_by_category.sort_by(|a, b| {
        b.amount
            .usd
            .partial_cmp(&a.amount.usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
        variable_by_category,
    })
}

/// Most recent transactions (active accounts) with their resolved flow type.
fn fetch_classified(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<ClassifiedTransaction>> {
    let limit = limit.clamp(1, 1000);
    let rules = load_rules(conn)?;
    let usd_rates = load_usd_rates(conn)?;

    let mut stmt = conn.prepare(
        "SELECT t.id, t.account_id, a.name, t.txn_date, t.description, t.amount, t.currency, \
                t.flow_override, a.account_type, t.category \
         FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE a.is_active = 1 \
         ORDER BY t.txn_date DESC, t.id DESC LIMIT ?1",
    )?;
    let mut ids: Vec<i64> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut inputs: Vec<FlowInput> = Vec::new();
    let mut rows = stmt.query(params![limit])?;
    while let Some(r) = rows.next()? {
        ids.push(r.get(0)?);
        names.push(r.get(2)?);
        inputs.push(FlowInput {
            account_id: r.get(1)?,
            date: r.get(3)?,
            description: r.get(4)?,
            amount: r.get(5)?,
            currency: r.get(6)?,
            override_opt: r.get(7)?,
            account_type: r.get(8)?,
            category: r.get(9)?,
        });
    }

    let flows = resolve_flow_types(&inputs, &rules, &usd_rates);

    let out = inputs
        .into_iter()
        .enumerate()
        .map(|(k, fi)| {
            let category = resolve_category(fi.category.as_deref(), &fi.description);
            ClassifiedTransaction {
                id: ids[k],
                account_id: fi.account_id,
                account_name: names[k].clone(),
                txn_date: fi.date,
                description: fi.description,
                amount: fi.amount,
                currency: fi.currency,
                flow_type: flows[k].as_str().to_string(),
                is_override: fi.override_opt.as_deref().and_then(FlowType::parse).is_some(),
                category,
            }
        })
        .collect();
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
                "chequing",
                &rules()
            ),
            FlowType::Variable
        );
    }

    #[test]
    fn rule_precedence_is_first_match() {
        // "mom" precedes "e-transfer", so the support payment is fixed, not an excluded transfer.
        assert_eq!(
            classify("INTERAC E-TRANSFER TO MOM", -800.0, None, "chequing", &rules()),
            FlowType::Fixed
        );
        assert_eq!(
            classify("E-TRANSFER TO LANDLORD", -1500.0, None, "chequing", &rules()),
            FlowType::Transfer
        );
    }

    #[test]
    fn sign_default_when_no_rule_matches() {
        assert_eq!(
            classify("STARBUCKS", -6.25, None, "chequing", &rules()),
            FlowType::Variable
        );
        assert_eq!(
            classify("PAYROLL", 2500.0, None, "chequing", &rules()),
            FlowType::Income
        );
    }

    #[test]
    fn inflow_into_investment_account_is_transfer_not_income() {
        // An RRSP contribution / security buy posts as a positive amount but is not earned income.
        assert_eq!(
            classify(
                "BLK US Equity Index Reg 1.6 units at $105",
                170.32,
                None,
                "rrsp",
                &rules()
            ),
            FlowType::Transfer
        );
        // Funding a brokerage account is likewise an internal move, not income.
        assert_eq!(
            classify(
                "Instant bank deposit to Robinhood",
                1000.0,
                None,
                "brokerage",
                &rules()
            ),
            FlowType::Transfer
        );
    }

    #[test]
    fn inflow_into_credit_account_is_transfer_not_income() {
        // A payment landing on a credit card pays it down — it is not income.
        assert_eq!(
            classify("PAYMENT FROM CHEQUING", 1500.0, None, "credit", &rules()),
            FlowType::Transfer
        );
    }

    #[test]
    fn outflow_from_credit_account_still_counts_as_spending() {
        // Purchases on the card (negative) remain real variable spending.
        assert_eq!(
            classify("CORNER STORE", -42.0, None, "credit", &rules()),
            FlowType::Variable
        );
    }

    #[test]
    fn inflow_into_cash_account_is_income() {
        assert_eq!(
            classify("ACME PAYROLL", 2500.0, None, "chequing", &rules()),
            FlowType::Income
        );
        assert_eq!(
            classify("INTEREST PAYMENT", 12.0, None, "savings", &rules()),
            FlowType::Income
        );
    }

    #[test]
    fn category_for_picks_specific_buckets() {
        assert_eq!(category_for("STARBUCKS #123 VANCOUVER"), Some("Coffee"));
        assert_eq!(category_for("NO FRILLS 4821"), Some("Groceries"));
        assert_eq!(category_for("UBER EATS"), Some("Dining"));
        assert_eq!(category_for("UBER TRIP HELP.UBER.COM"), Some("Transport"));
        assert_eq!(category_for("SHELL OIL 5567"), Some("Fuel"));
        assert_eq!(category_for("NETFLIX.COM"), Some("Subscriptions"));
        assert_eq!(category_for("AMAZON PRIME MEMBERSHIP"), Some("Subscriptions"));
        assert_eq!(category_for("AMAZON.CA ORDER"), Some("Shopping"));
        assert_eq!(category_for("ROGERS WIRELESS"), Some("Utilities"));
        assert_eq!(category_for("SOME UNKNOWN MERCHANT"), None);
    }

    #[test]
    fn resolve_category_prefers_stored_over_local() {
        // A stored (AI/manual) label wins over the local guess.
        assert_eq!(
            resolve_category(Some("Travel"), "STARBUCKS"),
            Some("Travel".to_string())
        );
        // Blank stored values fall back to the local guess.
        assert_eq!(
            resolve_category(Some("  "), "STARBUCKS"),
            Some("Coffee".to_string())
        );
        assert_eq!(resolve_category(None, "MYSTERY CO"), None);
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

    fn seed_account_typed(conn: &Connection, currency: &str, account_type: &str) -> i64 {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Acct', 'Inst', ?1, ?2, 'CA')",
            params![account_type, currency],
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
    fn compute_cashflow_excludes_investment_account_inflows() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let chequing = seed_account(&conn, "CAD");
        let rrsp = seed_account_typed(&conn, "CAD", "rrsp");
        let card = seed_account_typed(&conn, "CAD", "credit");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // Real salary in a cash account stays income.
        insert_txn(&conn, chequing, &today, "MICROSOFT PAYROLL", 4000.0, "CAD");
        // RRSP contribution buys post as positive amounts but are not earned income.
        insert_txn(&conn, rrsp, &today, "BLK US Equity Index Reg units at $105", 170.0, "CAD");
        insert_txn(&conn, rrsp, &today, "BLK EAFE Equity Index units at $35", 85.0, "CAD");
        // A one-legged credit-card payment (no matching cash debit synced) is still a transfer.
        insert_txn(&conn, card, &today, "PAYMENT FROM 07*60", 600.0, "CAD");
        // A purchase on the card is genuine variable spending.
        insert_txn(&conn, card, &today, "CORNER STORE", -50.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.txn_count, 5);
        // Income is the payroll only — not 4855 (payroll + RRSP buys + card payment).
        assert_eq!(s.income.cad, 4000.0);
        // The three inflows into the RRSP/credit accounts are excluded as transfers.
        assert_eq!(s.transfer_count, 3);
        // The card purchase still counts as spending.
        assert_eq!(s.variable.cad, 50.0);
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
        // The variable spend is bucketed by merchant category.
        assert_eq!(s.variable_by_category.len(), 1);
        assert_eq!(s.variable_by_category[0].category, "Groceries");
        assert_eq!(s.variable_by_category[0].amount.cad, 200.0);
        // USD figures use the 1.25 pivot.
        assert_eq!(s.income.usd, 4000.0);
        assert!((s.savings_rate - 0.8).abs() < 1e-9);
        assert!(!s.currency_warning);
    }

    #[test]
    fn brokerage_payee_counts_as_transfer_not_variable() {
        // A debit to a brokerage/exchange is an internal move, even with no matching inflow leg
        // synced. The back-filled rule (seed_txn_rules_v2) catches it by payee name.
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let acct = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        insert_txn(&conn, acct, &today, "WEALTHSIMPLE PAYMENTS INC", -2000.0, "CAD");
        insert_txn(&conn, acct, &today, "GROCERY STORE", -150.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.transfer_count, 1);
        assert_eq!(s.variable.cad, 150.0); // only the groceries, not the brokerage funding
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

    #[test]
    fn auto_detects_internal_transfer_between_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let a = seed_account(&conn, "CAD");
        let b = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // Equal-and-opposite move between the two accounts, with no transfer keyword in sight.
        insert_txn(&conn, a, &today, "OUTGOING FUNDS", -5000.0, "CAD");
        insert_txn(&conn, b, &today, "INCOMING FUNDS", 5000.0, "CAD");
        // A real paycheque and a real expense that must be left alone.
        insert_txn(&conn, a, &today, "MICROSOFT PAYROLL", 3000.0, "CAD");
        insert_txn(&conn, a, &today, "GROCERY STORE", -200.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.txn_count, 4);
        assert_eq!(s.transfer_count, 2); // both legs of the move excluded
        assert_eq!(s.income.cad, 3000.0); // payroll only, not 8000
        assert_eq!(s.variable.cad, 200.0); // groceries only, not 5200
        assert_eq!(s.fixed.cad, 0.0);
        assert_eq!(s.net_savings.cad, 2800.0);
    }

    #[test]
    fn auto_matches_card_payment_to_its_unlabelled_leg() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let chequing = seed_account(&conn, "CAD");
        let card = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // The card side hits the seeded "payment - thank you" transfer rule; the chequing side has
        // no keyword and would otherwise be counted as variable spending.
        insert_txn(&conn, card, &today, "PAYMENT - THANK YOU", 1500.0, "CAD");
        insert_txn(&conn, chequing, &today, "WWWPAY CARD AUTOPAY", -1500.0, "CAD");
        insert_txn(&conn, chequing, &today, "ACME SALARY", 4000.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.transfer_count, 2); // both legs excluded
        assert_eq!(s.variable.cad, 0.0); // the autopay is not spending
        assert_eq!(s.income.cad, 4000.0); // salary untouched
    }

    #[test]
    fn does_not_match_same_account_or_a_refund() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let acct = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // A purchase and its refund in the SAME account are not an internal transfer.
        insert_txn(&conn, acct, &today, "SHOE STORE", -120.0, "CAD");
        insert_txn(&conn, acct, &today, "SHOE STORE REFUND", 120.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.transfer_count, 0);
        assert_eq!(s.variable.cad, 120.0);
        assert_eq!(s.income.cad, 120.0);
    }

    #[test]
    fn does_not_match_transfers_outside_the_day_window() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.25, '2025-01-01')",
            [],
        )
        .unwrap();
        let a = seed_account(&conn, "CAD");
        let b = seed_account(&conn, "CAD");
        let recent = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let earlier = (chrono::Utc::now() - chrono::Duration::days(10))
            .format("%Y-%m-%d")
            .to_string();
        insert_txn(&conn, a, &earlier, "MOVE OUT", -750.0, "CAD");
        insert_txn(&conn, b, &recent, "MOVE IN", 750.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        // 10 days apart is beyond the 5-day pairing window, so the legs are not matched.
        assert_eq!(s.transfer_count, 0);
        assert_eq!(s.variable.cad, 750.0);
        assert_eq!(s.income.cad, 750.0);
    }

    #[test]
    fn auto_detects_cross_currency_internal_transfer() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        // 1 USD = 1.36 CAD.
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.36, '2025-01-01')",
            [],
        )
        .unwrap();
        let usd_acct = seed_account(&conn, "USD");
        let cad_acct = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // Move 5000 USD out of the US account; 6800 CAD (= 5000 USD at 1.36) lands in the CA account.
        insert_txn(&conn, usd_acct, &today, "OUTGOING WIRE", -5000.0, "USD");
        insert_txn(&conn, cad_acct, &today, "INCOMING WIRE", 6800.0, "CAD");
        // Real income and spending that must survive untouched.
        insert_txn(&conn, cad_acct, &today, "ACME PAYROLL", 3000.0, "CAD");
        insert_txn(&conn, usd_acct, &today, "CORNER STORE", -100.0, "USD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.transfer_count, 2); // both legs of the cross-currency move excluded
        assert!((s.income.cad - 3000.0).abs() < 0.01); // payroll only, not 9800
        assert!((s.variable.cad - 136.0).abs() < 0.01); // 100 USD of groceries only, not 6936
    }

    #[test]
    fn does_not_pair_cross_currency_beyond_tolerance() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO fx_rates (from_currency, to_currency, rate, rate_date) \
             VALUES ('USD', 'CAD', 1.36, '2025-01-01')",
            [],
        )
        .unwrap();
        let usd_acct = seed_account(&conn, "USD");
        let cad_acct = seed_account(&conn, "CAD");
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // 9000 CAD is ~6618 USD — nowhere near the 5000 USD debit, so these are not one transfer.
        insert_txn(&conn, usd_acct, &today, "OUTGOING WIRE", -5000.0, "USD");
        insert_txn(&conn, cad_acct, &today, "PAYCHEQUE", 9000.0, "CAD");

        let s = compute_cashflow(&conn, 30).unwrap();
        assert_eq!(s.transfer_count, 0);
        assert!((s.income.cad - 9000.0).abs() < 0.01); // the deposit is counted as income
        assert!((s.variable.cad - 6800.0).abs() < 0.01); // 5000 USD debit counted as spending
    }
}
