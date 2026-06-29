//! Progress metrics — forward-looking, motivating measures of wealth-building.
//!
//! Generic and neutral-default like the FIRE planner. Two metrics:
//!   * **Freedom runway** — months/years your net worth covers at your monthly burn. Net worth ÷
//!     monthly expenses. Expenses can be derived from real cashflow or overridden.
//!   * **Salary milestones** — net worth as a multiple of base salary (0.5×, 1×, 2×, 3×, 5×). Time
//!     gives compounding room, so these are far fairer for young high earners than backward-looking
//!     formulas. Each milestone shows progress and whether it's reached.
//! Inputs persist in `app_settings`; net worth + expenses come from the user's actual data.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::cashflow::compute_cashflow;
use crate::commands::net_worth::compute_net_worth_history;
use crate::db::AppDb;

const K_SALARY: &str = "progress_base_salary_usd";
const K_EXPENSES: &str = "progress_monthly_expenses_usd";
const K_YEARS: &str = "progress_years_earning";

const DEF_SALARY: f64 = 80_000.0;
const DEF_EXPENSES: f64 = 0.0; // 0 = derive from cashflow
const DEF_YEARS: f64 = 5.0;

const EXPENSE_WINDOW_DAYS: i64 = 90;
const DAYS_PER_MONTH: f64 = 30.44;
const MILESTONES: [f64; 5] = [0.5, 1.0, 2.0, 3.0, 5.0];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgressInputs {
    pub base_salary_usd: f64,
    /// Monthly living expenses. 0 = derive from the last ~90 days of cashflow.
    pub monthly_expenses_usd: f64,
    pub years_earning: f64,
}

impl Default for ProgressInputs {
    fn default() -> Self {
        Self {
            base_salary_usd: DEF_SALARY,
            monthly_expenses_usd: DEF_EXPENSES,
            years_earning: DEF_YEARS,
        }
    }
}

/// One salary-multiple milestone.
#[derive(Debug, Serialize, PartialEq)]
pub struct Milestone {
    pub multiple: f64,
    pub target_usd: f64,
    pub reached: bool,
    /// 0..1 toward this milestone.
    pub progress: f64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ProgressMetrics {
    pub inputs: ProgressInputs,
    pub current_usd: f64,
    /// Expenses used for the runway (derived or overridden).
    pub monthly_expenses_usd: f64,
    pub expenses_derived: bool,
    /// Net worth ÷ monthly expenses; null when expenses are unknown.
    pub freedom_months: Option<f64>,
    pub freedom_years: Option<f64>,
    /// Net worth ÷ base salary.
    pub salary_multiple: f64,
    pub milestones: Vec<Milestone>,
}

#[tauri::command]
pub fn get_progress_metrics(db: State<AppDb>) -> Result<ProgressMetrics, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let inputs = load_inputs(&conn).map_err(|e| e.to_string())?;
    let current = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    let derived = derive_monthly_expenses(&conn);
    Ok(compute_metrics(&inputs, current, derived))
}

#[tauri::command]
pub fn set_progress_inputs(
    db: State<AppDb>,
    inputs: ProgressInputs,
) -> Result<ProgressMetrics, String> {
    let inputs = sanitize(inputs)?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_inputs(&conn, &inputs).map_err(|e| e.to_string())?;
    let current = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    let derived = derive_monthly_expenses(&conn);
    Ok(compute_metrics(&inputs, current, derived))
}

fn current_net_worth_usd(conn: &Connection) -> rusqlite::Result<f64> {
    Ok(compute_net_worth_history(conn)?
        .last()
        .map(|p| p.total_usd)
        .unwrap_or(0.0))
}

/// Best-effort monthly burn from the last ~90 days of fixed + variable spend. None when there
/// aren't enough transactions to be meaningful.
fn derive_monthly_expenses(conn: &Connection) -> Option<f64> {
    let cf = compute_cashflow(conn, EXPENSE_WINDOW_DAYS).ok()?;
    let total = cf.fixed.usd + cf.variable.usd;
    if total <= 0.0 || cf.window_days <= 0 {
        return None;
    }
    Some(total / cf.window_days as f64 * DAYS_PER_MONTH)
}

/// `derived` is the cashflow-based monthly burn, used only when the override is 0.
fn compute_metrics(inputs: &ProgressInputs, current_usd: f64, derived: Option<f64>) -> ProgressMetrics {
    let (expenses, expenses_derived) = if inputs.monthly_expenses_usd > 0.0 {
        (Some(inputs.monthly_expenses_usd), false)
    } else {
        (derived, true)
    };
    let (freedom_months, freedom_years) = match expenses {
        Some(e) if e > 0.0 => {
            let m = current_usd / e;
            (Some(round2(m)), Some(round2(m / 12.0)))
        }
        _ => (None, None),
    };
    let salary = inputs.base_salary_usd;
    let salary_multiple = if salary > 0.0 { current_usd / salary } else { 0.0 };
    let milestones = MILESTONES
        .iter()
        .map(|&m| {
            let target = salary * m;
            Milestone {
                multiple: m,
                target_usd: round2(target),
                reached: target > 0.0 && current_usd >= target,
                progress: if target > 0.0 { (current_usd / target).clamp(0.0, 1.0) } else { 0.0 },
            }
        })
        .collect();

    ProgressMetrics {
        inputs: inputs.clone(),
        current_usd: round2(current_usd),
        monthly_expenses_usd: round2(expenses.unwrap_or(0.0)),
        expenses_derived,
        freedom_months,
        freedom_years,
        salary_multiple: round2(salary_multiple),
        milestones,
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn sanitize(mut a: ProgressInputs) -> Result<ProgressInputs, String> {
    if ![a.base_salary_usd, a.monthly_expenses_usd, a.years_earning]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err("Progress inputs must be finite numbers.".into());
    }
    a.base_salary_usd = a.base_salary_usd.max(0.0);
    a.monthly_expenses_usd = a.monthly_expenses_usd.max(0.0);
    a.years_earning = a.years_earning.max(0.0);
    Ok(a)
}

fn load_inputs(conn: &Connection) -> rusqlite::Result<ProgressInputs> {
    let d = ProgressInputs::default();
    Ok(ProgressInputs {
        base_salary_usd: get_f64(conn, K_SALARY, d.base_salary_usd)?,
        monthly_expenses_usd: get_f64(conn, K_EXPENSES, d.monthly_expenses_usd)?,
        years_earning: get_f64(conn, K_YEARS, d.years_earning)?,
    })
}

fn save_inputs(conn: &Connection, a: &ProgressInputs) -> rusqlite::Result<()> {
    let pairs: [(&str, String); 3] = [
        (K_SALARY, a.base_salary_usd.to_string()),
        (K_EXPENSES, a.monthly_expenses_usd.to_string()),
        (K_YEARS, a.years_earning.to_string()),
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

    #[test]
    fn freedom_runway_uses_override_when_set() {
        let inputs = ProgressInputs { base_salary_usd: 100_000.0, monthly_expenses_usd: 4_000.0, years_earning: 1.0 };
        let m = compute_metrics(&inputs, 96_000.0, Some(9_999.0));
        assert!(!m.expenses_derived);
        assert_eq!(m.monthly_expenses_usd, 4_000.0);
        assert_eq!(m.freedom_months, Some(24.0)); // 96k / 4k
        assert_eq!(m.freedom_years, Some(2.0));
    }

    #[test]
    fn freedom_runway_derives_when_override_zero() {
        let inputs = ProgressInputs { base_salary_usd: 100_000.0, monthly_expenses_usd: 0.0, years_earning: 1.0 };
        let m = compute_metrics(&inputs, 60_000.0, Some(5_000.0));
        assert!(m.expenses_derived);
        assert_eq!(m.freedom_months, Some(12.0));
    }

    #[test]
    fn no_expenses_means_no_runway() {
        let inputs = ProgressInputs { base_salary_usd: 100_000.0, monthly_expenses_usd: 0.0, years_earning: 1.0 };
        let m = compute_metrics(&inputs, 60_000.0, None);
        assert_eq!(m.freedom_months, None);
        assert_eq!(m.freedom_years, None);
    }

    #[test]
    fn milestones_track_salary_multiples() {
        let inputs = ProgressInputs { base_salary_usd: 100_000.0, monthly_expenses_usd: 4_000.0, years_earning: 2.0 };
        let m = compute_metrics(&inputs, 120_000.0, None);
        assert_eq!(m.salary_multiple, 1.2);
        // 0.5x and 1x reached; 2x/3x/5x not.
        assert!(m.milestones[0].reached && m.milestones[1].reached);
        assert!(!m.milestones[2].reached);
        assert_eq!(m.milestones[2].target_usd, 200_000.0);
        assert!((m.milestones[2].progress - 0.6).abs() < 1e-9);
    }

    #[test]
    fn defaults_persist_and_reload() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), ProgressInputs::default());
        let custom = ProgressInputs { base_salary_usd: 215_000.0, monthly_expenses_usd: 4_000.0, years_earning: 1.16 };
        save_inputs(&conn, &custom).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), custom);
    }
}
