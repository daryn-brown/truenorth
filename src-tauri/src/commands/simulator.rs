//! The "Seattle Transition" simulator.
//!
//! Projects net worth forward under two scenarios so the user can see what happens when the
//! cross-border move forces Job 2 to $0 and localizes the Microsoft salary to USD:
//!   * **Status quo** — both income streams continue at today's pace.
//!   * **Seattle** — at the transition month, the monthly contribution drops to the post-move
//!     (single-salary) pace.
//!
//! Both lines compound a shared assumed annual return, so the card shows the *cost* of dropping
//! Job 2 over the horizon while still reassuring the user that the trajectory keeps climbing.
//! Assumptions live in `app_settings` (seeded to the user's real figures on first read) and are
//! fully editable from the card.

use chrono::{Local, Months, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::net_worth::compute_net_worth_history;
use crate::db::AppDb;

const K_CURRENT_NET: &str = "sim_current_net_monthly_usd";
const K_CURRENT_EXP: &str = "sim_current_expenses_monthly_usd";
const K_SEATTLE_NET: &str = "sim_seattle_net_monthly_usd";
const K_SEATTLE_EXP: &str = "sim_seattle_expenses_monthly_usd";
const K_TRANSITION: &str = "sim_transition_months";
const K_HORIZON: &str = "sim_horizon_months";
const K_RETURN: &str = "sim_annual_return_pct";

// Defaults reflect the user's stated figures in USD; all are editable from the simulator card.
const DEF_CURRENT_NET: f64 = 11_337.0;
const DEF_CURRENT_EXP: f64 = 2_800.0;
const DEF_SEATTLE_NET: f64 = 9_300.0;
const DEF_SEATTLE_EXP: f64 = 2_800.0;
const DEF_TRANSITION_MONTHS: i64 = 6;
const DEF_HORIZON_MONTHS: i64 = 36;
const DEF_ANNUAL_RETURN_PCT: f64 = 6.0;

/// Guard rails so a fat-fingered horizon can't allocate a huge series or loop forever.
const MAX_HORIZON_MONTHS: i64 = 600;
const RETURN_PCT_BOUND: f64 = 50.0;

/// The editable levers behind the projection. Net/expense figures are monthly USD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SeattleAssumptions {
    /// Today's total net income across both jobs.
    pub current_net_monthly_usd: f64,
    /// Today's total spend (baseline burn + fixed commitments like the mom transfer).
    pub current_expenses_monthly_usd: f64,
    /// Post-move net income (localized Microsoft salary, Job 2 gone).
    pub seattle_net_monthly_usd: f64,
    /// Post-move spend (adjust for Seattle cost of living).
    pub seattle_expenses_monthly_usd: f64,
    /// Months from today until the move flips income to the Seattle figures.
    pub transition_months: i64,
    /// How far forward to project.
    pub horizon_months: i64,
    /// Assumed annual return applied to the running balance, compounded monthly.
    pub annual_return_pct: f64,
}

impl Default for SeattleAssumptions {
    fn default() -> Self {
        Self {
            current_net_monthly_usd: DEF_CURRENT_NET,
            current_expenses_monthly_usd: DEF_CURRENT_EXP,
            seattle_net_monthly_usd: DEF_SEATTLE_NET,
            seattle_expenses_monthly_usd: DEF_SEATTLE_EXP,
            transition_months: DEF_TRANSITION_MONTHS,
            horizon_months: DEF_HORIZON_MONTHS,
            annual_return_pct: DEF_ANNUAL_RETURN_PCT,
        }
    }
}

/// One month on the projection. `month` 0 is "today" (both scenarios start equal).
#[derive(Debug, Serialize, PartialEq)]
pub struct ProjectionPoint {
    pub month: i64,
    /// ISO date YYYY-MM-DD.
    pub date: String,
    pub current_usd: f64,
    pub seattle_usd: f64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SeattleProjection {
    pub start_usd: f64,
    pub start_date: String,
    /// The month the Seattle scenario diverges (start_date + transition_months).
    pub transition_date: String,
    /// Monthly amount added to net worth today (current_net − current_expenses).
    pub current_monthly_contribution_usd: f64,
    /// Monthly amount added after the move (seattle_net − seattle_expenses).
    pub seattle_monthly_contribution_usd: f64,
    pub current_end_usd: f64,
    pub seattle_end_usd: f64,
    /// seattle_end − current_end (negative = the cost of dropping Job 2 over the horizon).
    pub end_gap_usd: f64,
    pub points: Vec<ProjectionPoint>,
    /// Echoed back so the card can render the editor without a second call.
    pub assumptions: SeattleAssumptions,
}

/// Project net worth forward under the status-quo and Seattle scenarios.
#[tauri::command]
pub fn get_seattle_projection(db: State<AppDb>) -> Result<SeattleProjection, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let assumptions = load_assumptions(&conn).map_err(|e| e.to_string())?;
    let start_usd = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    Ok(compute_projection(
        start_usd,
        Local::now().date_naive(),
        &assumptions,
    ))
}

/// Persist edited assumptions and return the recomputed projection.
#[tauri::command]
pub fn set_seattle_assumptions(
    db: State<AppDb>,
    assumptions: SeattleAssumptions,
) -> Result<SeattleProjection, String> {
    let assumptions = sanitize(assumptions)?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_assumptions(&conn, &assumptions).map_err(|e| e.to_string())?;
    let start_usd = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    Ok(compute_projection(
        start_usd,
        Local::now().date_naive(),
        &assumptions,
    ))
}

fn current_net_worth_usd(conn: &Connection) -> rusqlite::Result<f64> {
    Ok(compute_net_worth_history(conn)?
        .last()
        .map(|p| p.total_usd)
        .unwrap_or(0.0))
}

fn compute_projection(
    start_usd: f64,
    start_date: NaiveDate,
    assumptions: &SeattleAssumptions,
) -> SeattleProjection {
    let current_contrib =
        assumptions.current_net_monthly_usd - assumptions.current_expenses_monthly_usd;
    let seattle_contrib =
        assumptions.seattle_net_monthly_usd - assumptions.seattle_expenses_monthly_usd;
    let monthly_growth = (1.0 + assumptions.annual_return_pct / 100.0).powf(1.0 / 12.0) - 1.0;
    let horizon = assumptions.horizon_months.clamp(1, MAX_HORIZON_MONTHS);
    let transition = assumptions.transition_months.clamp(0, MAX_HORIZON_MONTHS);

    let mut current_bal = start_usd;
    let mut seattle_bal = start_usd;
    let mut points = Vec::with_capacity(horizon as usize + 1);
    points.push(ProjectionPoint {
        month: 0,
        date: add_months(start_date, 0),
        current_usd: round2(start_usd),
        seattle_usd: round2(start_usd),
    });

    for month in 1..=horizon {
        current_bal = current_bal * (1.0 + monthly_growth) + current_contrib;
        // The Seattle line tracks the status quo until the move, then switches pace.
        let seattle_month_contrib = if month <= transition {
            current_contrib
        } else {
            seattle_contrib
        };
        seattle_bal = seattle_bal * (1.0 + monthly_growth) + seattle_month_contrib;

        points.push(ProjectionPoint {
            month,
            date: add_months(start_date, month),
            current_usd: round2(current_bal),
            seattle_usd: round2(seattle_bal),
        });
    }

    let current_end_usd = points.last().map(|p| p.current_usd).unwrap_or(start_usd);
    let seattle_end_usd = points.last().map(|p| p.seattle_usd).unwrap_or(start_usd);

    SeattleProjection {
        start_usd: round2(start_usd),
        start_date: add_months(start_date, 0),
        transition_date: add_months(start_date, transition),
        current_monthly_contribution_usd: round2(current_contrib),
        seattle_monthly_contribution_usd: round2(seattle_contrib),
        current_end_usd,
        seattle_end_usd,
        end_gap_usd: round2(seattle_end_usd - current_end_usd),
        points,
        assumptions: assumptions.clone(),
    }
}

/// Add `months` calendar months to `date`, formatted YYYY-MM-DD. Falls back to the start date on
/// the (practically impossible) overflow.
fn add_months(date: NaiveDate, months: i64) -> String {
    let bumped = date
        .checked_add_months(Months::new(months.max(0) as u32))
        .unwrap_or(date);
    bumped.format("%Y-%m-%d").to_string()
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn sanitize(mut a: SeattleAssumptions) -> Result<SeattleAssumptions, String> {
    let all_finite = [
        a.current_net_monthly_usd,
        a.current_expenses_monthly_usd,
        a.seattle_net_monthly_usd,
        a.seattle_expenses_monthly_usd,
        a.annual_return_pct,
    ]
    .iter()
    .all(|v| v.is_finite());
    if !all_finite {
        return Err("Assumptions must be finite numbers.".into());
    }
    a.transition_months = a.transition_months.clamp(0, MAX_HORIZON_MONTHS);
    a.horizon_months = a.horizon_months.clamp(1, MAX_HORIZON_MONTHS);
    a.annual_return_pct = a
        .annual_return_pct
        .clamp(-RETURN_PCT_BOUND, RETURN_PCT_BOUND);
    Ok(a)
}

fn load_assumptions(conn: &Connection) -> rusqlite::Result<SeattleAssumptions> {
    let d = SeattleAssumptions::default();
    Ok(SeattleAssumptions {
        current_net_monthly_usd: get_f64(conn, K_CURRENT_NET, d.current_net_monthly_usd)?,
        current_expenses_monthly_usd: get_f64(conn, K_CURRENT_EXP, d.current_expenses_monthly_usd)?,
        seattle_net_monthly_usd: get_f64(conn, K_SEATTLE_NET, d.seattle_net_monthly_usd)?,
        seattle_expenses_monthly_usd: get_f64(conn, K_SEATTLE_EXP, d.seattle_expenses_monthly_usd)?,
        transition_months: get_i64(conn, K_TRANSITION, d.transition_months)?,
        horizon_months: get_i64(conn, K_HORIZON, d.horizon_months)?,
        annual_return_pct: get_f64(conn, K_RETURN, d.annual_return_pct)?,
    })
}

fn save_assumptions(conn: &Connection, a: &SeattleAssumptions) -> rusqlite::Result<()> {
    let pairs: [(&str, String); 7] = [
        (K_CURRENT_NET, a.current_net_monthly_usd.to_string()),
        (K_CURRENT_EXP, a.current_expenses_monthly_usd.to_string()),
        (K_SEATTLE_NET, a.seattle_net_monthly_usd.to_string()),
        (K_SEATTLE_EXP, a.seattle_expenses_monthly_usd.to_string()),
        (K_TRANSITION, a.transition_months.to_string()),
        (K_HORIZON, a.horizon_months.to_string()),
        (K_RETURN, a.annual_return_pct.to_string()),
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

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

fn get_f64(conn: &Connection, key: &str, default: f64) -> rusqlite::Result<f64> {
    Ok(get_setting(conn, key)?
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default))
}

fn get_i64(conn: &Connection, key: &str, default: i64) -> rusqlite::Result<i64> {
    Ok(get_setting(conn, key)?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{apply_schema, seed_defaults};

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn assumptions(
        current_net: f64,
        current_exp: f64,
        seattle_net: f64,
        seattle_exp: f64,
        transition: i64,
        horizon: i64,
        annual_return: f64,
    ) -> SeattleAssumptions {
        SeattleAssumptions {
            current_net_monthly_usd: current_net,
            current_expenses_monthly_usd: current_exp,
            seattle_net_monthly_usd: seattle_net,
            seattle_expenses_monthly_usd: seattle_exp,
            transition_months: transition,
            horizon_months: horizon,
            annual_return_pct: annual_return,
        }
    }

    #[test]
    fn status_quo_is_linear_when_growth_is_zero() {
        // contrib = 11_337 - 2_800 = 8_537/mo; transition past the horizon, so Seattle == current.
        let a = assumptions(11_337.0, 2_800.0, 9_300.0, 2_800.0, 6, 3, 0.0);
        let p = compute_projection(64_000.0, date("2026-01-01"), &a);

        assert_eq!(p.points.len(), 4); // month 0..3
        assert_eq!(p.current_monthly_contribution_usd, 8_537.0);
        assert_eq!(p.points[3].current_usd, 64_000.0 + 3.0 * 8_537.0);
        assert_eq!(p.points[3].seattle_usd, p.points[3].current_usd);
        assert_eq!(p.current_end_usd, 89_611.0);
        assert_eq!(p.end_gap_usd, 0.0);
    }

    #[test]
    fn seattle_tracks_status_quo_until_the_move_then_diverges() {
        // current contrib 8_537/mo, seattle contrib 9_300 - 2_800 = 6_500/mo, move after month 2.
        let a = assumptions(11_337.0, 2_800.0, 9_300.0, 2_800.0, 2, 4, 0.0);
        let p = compute_projection(64_000.0, date("2026-01-01"), &a);

        assert_eq!(p.seattle_monthly_contribution_usd, 6_500.0);
        // Identical through the transition month.
        assert_eq!(p.points[2].current_usd, p.points[2].seattle_usd);
        // Then the Seattle line grows slower.
        assert!(p.points[3].seattle_usd < p.points[3].current_usd);

        let current_end = 64_000.0 + 4.0 * 8_537.0;
        let seattle_end = 64_000.0 + 2.0 * 8_537.0 + 2.0 * 6_500.0;
        assert_eq!(p.current_end_usd, current_end);
        assert_eq!(p.seattle_end_usd, seattle_end);
        assert_eq!(p.end_gap_usd, round2(seattle_end - current_end));
        assert!(p.end_gap_usd < 0.0);
    }

    #[test]
    fn monthly_growth_compounds_to_the_annual_rate() {
        // No contributions: a 12% annual return compounded monthly returns the balance * 1.12.
        let a = assumptions(0.0, 0.0, 0.0, 0.0, 0, 12, 12.0);
        let p = compute_projection(1_000.0, date("2026-01-01"), &a);
        assert!((p.current_end_usd - 1_120.0).abs() < 0.5);
    }

    #[test]
    fn transition_date_is_start_plus_months() {
        let a = assumptions(11_337.0, 2_800.0, 9_300.0, 2_800.0, 6, 12, 6.0);
        let p = compute_projection(50_000.0, date("2026-01-15"), &a);
        assert_eq!(p.start_date, "2026-01-15");
        assert_eq!(p.transition_date, "2026-07-15");
    }

    #[test]
    fn sanitize_clamps_bounds_and_rejects_nonfinite() {
        let mut a = assumptions(11_337.0, 2_800.0, 9_300.0, 2_800.0, -4, 0, 999.0);
        let clean = sanitize(a.clone()).unwrap();
        assert_eq!(clean.transition_months, 0);
        assert_eq!(clean.horizon_months, 1);
        assert_eq!(clean.annual_return_pct, RETURN_PCT_BOUND);

        a.current_net_monthly_usd = f64::NAN;
        assert!(sanitize(a).is_err());
    }

    #[test]
    fn assumptions_default_then_persist_and_reload() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();

        // Nothing stored yet → the user's seeded figures.
        assert_eq!(
            load_assumptions(&conn).unwrap(),
            SeattleAssumptions::default()
        );

        let custom = assumptions(12_000.0, 3_000.0, 9_000.0, 3_500.0, 8, 48, 5.5);
        save_assumptions(&conn, &custom).unwrap();
        assert_eq!(load_assumptions(&conn).unwrap(), custom);

        // A garbage override falls back to the default for that field instead of breaking.
        conn.execute(
            "UPDATE app_settings SET value = 'oops' WHERE key = ?1",
            params![K_RETURN],
        )
        .unwrap();
        assert_eq!(
            load_assumptions(&conn).unwrap().annual_return_pct,
            DEF_ANNUAL_RETURN_PCT
        );
    }
}
