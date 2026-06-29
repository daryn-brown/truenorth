//! The FIRE planner — a generic Financial Independence / Retire Early calculator.
//!
//! Unlike the Seattle simulator (which is personalised), this card ships **neutral defaults** so
//! every install starts blank-but-sensible, then the user dials in their own numbers, which
//! persist locally in `app_settings`. From a handful of inputs it derives:
//!   * **FIRE number** — annual expenses / safe-withdrawal-rate (the 4% rule by default).
//!   * **CoastFIRE number** — what you'd need invested *today* to coast to the FIRE number by your
//!     retirement age with no further contributions.
//!   * **Timelines** — projecting current net worth + monthly contributions at the expected return,
//!     the age/date you hit CoastFIRE and full FIRE.
//!
//! Current portfolio comes from net-worth history; the monthly contribution defaults to the user's
//! own ~30-day net-worth pace (so it's "their numbers" automatically) but can be overridden.

use chrono::{Duration, Local, Months, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::net_worth::{compute_net_worth_history, NetWorthHistoryPoint};
use crate::db::AppDb;

const K_AGE: &str = "fire_current_age";
const K_EXPENSES: &str = "fire_annual_expenses_usd";
const K_SWR: &str = "fire_swr_pct";
const K_RETURN: &str = "fire_annual_return_pct";
const K_RETIRE_AGE: &str = "fire_retirement_age";
const K_CONTRIB: &str = "fire_monthly_contribution_usd";

// Neutral, non-personal starting points so the card is useful before anyone touches it.
const DEF_AGE: f64 = 30.0;
const DEF_EXPENSES: f64 = 40_000.0;
const DEF_SWR_PCT: f64 = 4.0;
const DEF_RETURN_PCT: f64 = 7.0;
const DEF_RETIRE_AGE: f64 = 65.0;
// 0 = "derive from my net-worth pace" rather than a fixed override.
const DEF_CONTRIB: f64 = 0.0;

const ROLLING_WINDOW_DAYS: i64 = 30;
/// Cap the projection so an extreme set of inputs can't loop forever; ~80 years of months.
const MAX_PROJECTION_MONTHS: i64 = 960;
const SWR_BOUND: f64 = 20.0;
const RETURN_PCT_BOUND: f64 = 50.0;
const MAX_AGE: f64 = 120.0;

/// User-editable levers behind the FIRE plan. Money is monthly/annual USD; ages are whole years.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FireInputs {
    pub current_age: f64,
    /// Target annual spend in retirement (drives the FIRE number).
    pub annual_expenses_usd: f64,
    /// Safe withdrawal rate, e.g. 4.0 for the 4% rule.
    pub swr_pct: f64,
    /// Expected long-run annual return, compounded monthly.
    pub annual_return_pct: f64,
    /// Traditional retirement age used for the CoastFIRE discount.
    pub retirement_age: f64,
    /// Monthly amount invested. 0 = derive from the user's ~30-day net-worth pace.
    pub monthly_contribution_usd: f64,
}

impl Default for FireInputs {
    fn default() -> Self {
        Self {
            current_age: DEF_AGE,
            annual_expenses_usd: DEF_EXPENSES,
            swr_pct: DEF_SWR_PCT,
            annual_return_pct: DEF_RETURN_PCT,
            retirement_age: DEF_RETIRE_AGE,
            monthly_contribution_usd: DEF_CONTRIB,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FirePlan {
    /// Echoed back so the editor renders without a second call.
    pub inputs: FireInputs,
    pub current_usd: f64,
    /// Contribution actually used (the override, or the derived pace).
    pub monthly_contribution_usd: f64,
    /// True when the contribution came from the net-worth pace rather than a manual override.
    pub contribution_is_derived: bool,
    /// Full-FIRE target: expenses / SWR.
    pub fire_number: f64,
    /// What you'd need invested today to coast to fire_number by retirement.
    pub coast_number: f64,
    /// 0..1 toward each milestone.
    pub fire_progress: f64,
    pub coast_progress: f64,
    pub already_fire: bool,
    pub already_coast: bool,
    /// Months/age/date to hit CoastFIRE at the current pace; None if unreachable by 120.
    pub coast_months: Option<i64>,
    pub coast_age: Option<f64>,
    pub coast_date: Option<String>,
    /// Months/age/date to hit full FIRE at the current pace; None if unreachable by 120.
    pub fire_months: Option<i64>,
    pub fire_age: Option<f64>,
    pub fire_date: Option<String>,
}

/// Current FIRE plan from saved inputs + live net worth.
#[tauri::command]
pub fn get_fire_plan(db: State<AppDb>) -> Result<FirePlan, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let inputs = load_inputs(&conn).map_err(|e| e.to_string())?;
    let history = compute_net_worth_history(&conn).map_err(|e| e.to_string())?;
    Ok(compute_plan(&inputs, &history, Local::now().date_naive()))
}

/// Persist edited inputs and return the recomputed plan.
#[tauri::command]
pub fn set_fire_inputs(db: State<AppDb>, inputs: FireInputs) -> Result<FirePlan, String> {
    let inputs = sanitize(inputs)?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_inputs(&conn, &inputs).map_err(|e| e.to_string())?;
    let history = compute_net_worth_history(&conn).map_err(|e| e.to_string())?;
    Ok(compute_plan(&inputs, &history, Local::now().date_naive()))
}

fn compute_plan(
    inputs: &FireInputs,
    history: &[NetWorthHistoryPoint],
    today: NaiveDate,
) -> FirePlan {
    let current_usd = history.last().map(|p| p.total_usd).unwrap_or(0.0);

    let (monthly_contribution_usd, contribution_is_derived) =
        if inputs.monthly_contribution_usd > 0.0 {
            (inputs.monthly_contribution_usd, false)
        } else {
            (rolling_monthly_pace(history).max(0.0), true)
        };

    let fire_number = if inputs.swr_pct > 0.0 {
        inputs.annual_expenses_usd / (inputs.swr_pct / 100.0)
    } else {
        0.0
    };
    let years_to_retire = (inputs.retirement_age - inputs.current_age).max(0.0);
    let coast_number =
        fire_number / (1.0 + inputs.annual_return_pct / 100.0).powf(years_to_retire);

    let fire_progress = ratio(current_usd, fire_number);
    let coast_progress = ratio(current_usd, coast_number);
    let already_fire = current_usd >= fire_number && fire_number > 0.0;
    let already_coast = current_usd >= coast_number && coast_number > 0.0;

    let monthly_growth = (1.0 + inputs.annual_return_pct / 100.0).powf(1.0 / 12.0) - 1.0;
    let coast_months = months_to_threshold(
        current_usd,
        monthly_contribution_usd,
        monthly_growth,
        coast_number,
    );
    let fire_months = months_to_threshold(
        current_usd,
        monthly_contribution_usd,
        monthly_growth,
        fire_number,
    );

    FirePlan {
        inputs: inputs.clone(),
        current_usd: round2(current_usd),
        monthly_contribution_usd: round2(monthly_contribution_usd),
        contribution_is_derived,
        fire_number: round2(fire_number),
        coast_number: round2(coast_number),
        fire_progress,
        coast_progress,
        already_fire,
        already_coast,
        coast_months,
        coast_age: coast_months.map(|m| year_age(inputs.current_age, m)),
        coast_date: coast_months.map(|m| add_months(today, m)),
        fire_months,
        fire_age: fire_months.map(|m| year_age(inputs.current_age, m)),
        fire_date: fire_months.map(|m| add_months(today, m)),
    }
}

/// First month where a balance compounding at `monthly_growth` plus `contribution` reaches
/// `target`. 0 if already there; None if a flat/zero pace never gets there within the cap.
fn months_to_threshold(
    start: f64,
    contribution: f64,
    monthly_growth: f64,
    target: f64,
) -> Option<i64> {
    if target <= 0.0 {
        return None;
    }
    if start >= target {
        return Some(0);
    }
    let mut bal = start;
    for month in 1..=MAX_PROJECTION_MONTHS {
        bal = bal * (1.0 + monthly_growth) + contribution;
        if bal >= target {
            return Some(month);
        }
    }
    None
}

/// Net-worth change per 30 days over roughly the trailing window. 0 with too little history.
fn rolling_monthly_pace(history: &[NetWorthHistoryPoint]) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }
    let last = history.last().unwrap();
    let last_date = match NaiveDate::parse_from_str(&last.date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return 0.0,
    };
    let window_start = last_date - Duration::days(ROLLING_WINDOW_DAYS);
    let anchor = history[..history.len() - 1]
        .iter()
        .filter_map(|p| NaiveDate::parse_from_str(&p.date, "%Y-%m-%d").ok().map(|d| (p, d)))
        .min_by_key(|(_, d)| (*d - window_start).num_days().abs());
    match anchor {
        Some((p, d)) => {
            let span = (last_date - d).num_days();
            if span <= 0 {
                0.0
            } else {
                (last.total_usd - p.total_usd) / span as f64 * ROLLING_WINDOW_DAYS as f64
            }
        }
        None => 0.0,
    }
}

fn ratio(value: f64, target: f64) -> f64 {
    if target > 0.0 {
        (value / target).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn year_age(current_age: f64, months: i64) -> f64 {
    ((current_age + months as f64 / 12.0) * 10.0).round() / 10.0
}

fn add_months(date: NaiveDate, months: i64) -> String {
    date.checked_add_months(Months::new(months.max(0) as u32))
        .unwrap_or(date)
        .format("%Y-%m-%d")
        .to_string()
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn sanitize(mut a: FireInputs) -> Result<FireInputs, String> {
    let finite = [
        a.current_age,
        a.annual_expenses_usd,
        a.swr_pct,
        a.annual_return_pct,
        a.retirement_age,
        a.monthly_contribution_usd,
    ]
    .iter()
    .all(|v| v.is_finite());
    if !finite {
        return Err("FIRE inputs must be finite numbers.".into());
    }
    a.current_age = a.current_age.clamp(0.0, MAX_AGE);
    a.retirement_age = a.retirement_age.clamp(0.0, MAX_AGE);
    a.annual_expenses_usd = a.annual_expenses_usd.max(0.0);
    a.swr_pct = a.swr_pct.clamp(0.1, SWR_BOUND);
    a.annual_return_pct = a.annual_return_pct.clamp(-RETURN_PCT_BOUND, RETURN_PCT_BOUND);
    a.monthly_contribution_usd = a.monthly_contribution_usd.max(0.0);
    Ok(a)
}

fn load_inputs(conn: &Connection) -> rusqlite::Result<FireInputs> {
    let d = FireInputs::default();
    Ok(FireInputs {
        current_age: get_f64(conn, K_AGE, d.current_age)?,
        annual_expenses_usd: get_f64(conn, K_EXPENSES, d.annual_expenses_usd)?,
        swr_pct: get_f64(conn, K_SWR, d.swr_pct)?,
        annual_return_pct: get_f64(conn, K_RETURN, d.annual_return_pct)?,
        retirement_age: get_f64(conn, K_RETIRE_AGE, d.retirement_age)?,
        monthly_contribution_usd: get_f64(conn, K_CONTRIB, d.monthly_contribution_usd)?,
    })
}

fn save_inputs(conn: &Connection, a: &FireInputs) -> rusqlite::Result<()> {
    let pairs: [(&str, String); 6] = [
        (K_AGE, a.current_age.to_string()),
        (K_EXPENSES, a.annual_expenses_usd.to_string()),
        (K_SWR, a.swr_pct.to_string()),
        (K_RETURN, a.annual_return_pct.to_string()),
        (K_RETIRE_AGE, a.retirement_age.to_string()),
        (K_CONTRIB, a.monthly_contribution_usd.to_string()),
    ];
    for (key, value) in pairs {
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) \
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value],
        )?;
    }
    Ok(())
}

fn get_f64(conn: &Connection, key: &str, default: f64) -> rusqlite::Result<f64> {
    Ok(conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{apply_schema, seed_defaults};

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn point(d: &str, usd: f64) -> NetWorthHistoryPoint {
        NetWorthHistoryPoint {
            date: d.to_string(),
            total_usd: usd,
            total_cad: usd * 1.3,
        }
    }

    #[test]
    fn fire_number_is_expenses_over_swr() {
        let inputs = FireInputs {
            annual_expenses_usd: 40_000.0,
            swr_pct: 4.0,
            ..Default::default()
        };
        let plan = compute_plan(&inputs, &[point("2026-01-01", 0.0)], date("2026-01-01"));
        assert_eq!(plan.fire_number, 1_000_000.0);
        // CoastFIRE is the discounted target: 1M / 1.07^35 < FIRE number.
        assert!(plan.coast_number < plan.fire_number);
        assert!(plan.coast_number > 0.0);
    }

    #[test]
    fn manual_contribution_overrides_derived_pace() {
        let inputs = FireInputs {
            monthly_contribution_usd: 2_000.0,
            ..Default::default()
        };
        let plan = compute_plan(&inputs, &[point("2026-01-01", 100_000.0)], date("2026-01-01"));
        assert_eq!(plan.monthly_contribution_usd, 2_000.0);
        assert!(!plan.contribution_is_derived);
    }

    #[test]
    fn contribution_derives_from_rolling_pace() {
        // +$6,000 over 30 days → $6,000/mo pace, no override.
        let history = vec![point("2026-01-01", 50_000.0), point("2026-01-31", 56_000.0)];
        let plan = compute_plan(&FireInputs::default(), &history, date("2026-01-31"));
        assert!(plan.contribution_is_derived);
        assert_eq!(plan.monthly_contribution_usd, 6_000.0);
    }

    #[test]
    fn reaches_fire_when_already_funded() {
        let inputs = FireInputs {
            annual_expenses_usd: 40_000.0,
            swr_pct: 4.0,
            ..Default::default()
        };
        let plan = compute_plan(&inputs, &[point("2026-01-01", 1_200_000.0)], date("2026-01-01"));
        assert!(plan.already_fire);
        assert!(plan.already_coast);
        assert_eq!(plan.fire_months, Some(0));
        assert_eq!(plan.fire_progress, 1.0);
    }

    #[test]
    fn flat_pace_has_no_eta() {
        let inputs = FireInputs {
            monthly_contribution_usd: 0.0,
            annual_return_pct: 0.0,
            ..Default::default()
        };
        // Single point → 0 pace, 0 return → never reaches the target.
        let plan = compute_plan(&inputs, &[point("2026-01-01", 10_000.0)], date("2026-01-01"));
        assert_eq!(plan.fire_months, None);
        assert_eq!(plan.fire_age, None);
    }

    #[test]
    fn age_advances_with_timeline() {
        let inputs = FireInputs {
            current_age: 30.0,
            monthly_contribution_usd: 5_000.0,
            ..Default::default()
        };
        let plan = compute_plan(&inputs, &[point("2026-01-01", 100_000.0)], date("2026-01-01"));
        assert!(plan.fire_age.unwrap() > 30.0);
        assert!(plan.fire_date.is_some());
    }

    #[test]
    fn sanitize_clamps_and_rejects_nonfinite() {
        let mut a = FireInputs {
            swr_pct: 99.0,
            annual_return_pct: 999.0,
            ..Default::default()
        };
        let clean = sanitize(a.clone()).unwrap();
        assert_eq!(clean.swr_pct, SWR_BOUND);
        assert_eq!(clean.annual_return_pct, RETURN_PCT_BOUND);
        a.annual_expenses_usd = f64::NAN;
        assert!(sanitize(a).is_err());
    }

    #[test]
    fn defaults_persist_and_reload() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), FireInputs::default());

        let custom = FireInputs {
            current_age: 24.0,
            annual_expenses_usd: 57_600.0,
            swr_pct: 4.0,
            annual_return_pct: 8.0,
            retirement_age: 60.0,
            monthly_contribution_usd: 4_780.0,
        };
        save_inputs(&conn, &custom).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), custom);
    }
}
