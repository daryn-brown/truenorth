//! Wealth benchmark — how net worth compares to income-based formulas.
//!
//! Generic, neutral-default like the FIRE planner. From age + gross income it computes:
//!   * **Expected net worth** (The Millionaire Next Door): age × income / 10.
//!   * **PAW/UAW status**: ≥2× expected = Prodigious Accumulator, ≤0.5× = Under-Accumulator.
//!   * **Under-40 adjusted target**: age × income / (10 + (40 − age)) — fairer for young high
//!     earners whose income spiked recently (the divisor shrinks back to 10 at 40+).
//!   * **Accumulation velocity**: net worth retained per year of earning.
//! Inputs persist in `app_settings`; net worth comes from history.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::net_worth::compute_net_worth_history;
use crate::db::AppDb;

const K_AGE: &str = "wealth_current_age";
const K_INCOME: &str = "wealth_gross_income_usd";
const K_YEARS: &str = "wealth_years_earning";

const DEF_AGE: f64 = 30.0;
const DEF_INCOME: f64 = 60_000.0;
const DEF_YEARS: f64 = 5.0;

const PAW_MULTIPLE: f64 = 2.0;
const UAW_MULTIPLE: f64 = 0.5;
const MAX_AGE: f64 = 120.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WealthInputs {
    pub current_age: f64,
    pub gross_income_usd: f64,
    /// Years actually earning at roughly this level — denominator for velocity.
    pub years_earning: f64,
}

impl Default for WealthInputs {
    fn default() -> Self {
        Self {
            current_age: DEF_AGE,
            gross_income_usd: DEF_INCOME,
            years_earning: DEF_YEARS,
        }
    }
}

/// Where the current net worth lands vs. the expected figure.
#[derive(Debug, Serialize, PartialEq)]
pub enum AccumulatorStatus {
    Under,   // ≤ 0.5× expected
    Average, // between
    Prodigious, // ≥ 2× expected
}

#[derive(Debug, Serialize, PartialEq)]
pub struct WealthBenchmark {
    pub inputs: WealthInputs,
    pub current_usd: f64,
    /// Millionaire Next Door expected net worth.
    pub expected_usd: f64,
    /// 2× expected — the PAW bar.
    pub prodigious_usd: f64,
    /// Under-40 adjusted, fairer target.
    pub adjusted_usd: f64,
    /// current / expected (0 when income is 0).
    pub ratio: f64,
    pub status: AccumulatorStatus,
    /// Net worth retained per year of earning.
    pub velocity_usd: f64,
    /// 0..1 toward the adjusted target.
    pub adjusted_progress: f64,
}

#[tauri::command]
pub fn get_wealth_benchmark(db: State<AppDb>) -> Result<WealthBenchmark, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let inputs = load_inputs(&conn).map_err(|e| e.to_string())?;
    let current = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    Ok(compute_benchmark(&inputs, current))
}

#[tauri::command]
pub fn set_wealth_inputs(db: State<AppDb>, inputs: WealthInputs) -> Result<WealthBenchmark, String> {
    let inputs = sanitize(inputs)?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_inputs(&conn, &inputs).map_err(|e| e.to_string())?;
    let current = current_net_worth_usd(&conn).map_err(|e| e.to_string())?;
    Ok(compute_benchmark(&inputs, current))
}

fn current_net_worth_usd(conn: &Connection) -> rusqlite::Result<f64> {
    Ok(compute_net_worth_history(conn)?
        .last()
        .map(|p| p.total_usd)
        .unwrap_or(0.0))
}

fn compute_benchmark(inputs: &WealthInputs, current_usd: f64) -> WealthBenchmark {
    let expected = inputs.current_age * inputs.gross_income_usd / 10.0;
    // (40 - age) floors at 0, so 40+ collapses to the classic /10.
    let adj_div = 10.0 + (40.0 - inputs.current_age).max(0.0);
    let adjusted = inputs.current_age * inputs.gross_income_usd / adj_div;
    let ratio = if expected > 0.0 { current_usd / expected } else { 0.0 };
    let status = if ratio >= PAW_MULTIPLE {
        AccumulatorStatus::Prodigious
    } else if ratio <= UAW_MULTIPLE {
        AccumulatorStatus::Under
    } else {
        AccumulatorStatus::Average
    };
    let velocity = if inputs.years_earning > 0.0 {
        current_usd / inputs.years_earning
    } else {
        0.0
    };
    let adjusted_progress = if adjusted > 0.0 {
        (current_usd / adjusted).clamp(0.0, 1.0)
    } else {
        0.0
    };

    WealthBenchmark {
        inputs: inputs.clone(),
        current_usd: round2(current_usd),
        expected_usd: round2(expected),
        prodigious_usd: round2(expected * PAW_MULTIPLE),
        adjusted_usd: round2(adjusted),
        ratio: round2(ratio),
        status,
        velocity_usd: round2(velocity),
        adjusted_progress,
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn sanitize(mut a: WealthInputs) -> Result<WealthInputs, String> {
    if ![a.current_age, a.gross_income_usd, a.years_earning]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err("Wealth inputs must be finite numbers.".into());
    }
    a.current_age = a.current_age.clamp(0.0, MAX_AGE);
    a.gross_income_usd = a.gross_income_usd.max(0.0);
    a.years_earning = a.years_earning.max(0.0);
    Ok(a)
}

fn load_inputs(conn: &Connection) -> rusqlite::Result<WealthInputs> {
    let d = WealthInputs::default();
    Ok(WealthInputs {
        current_age: get_f64(conn, K_AGE, d.current_age)?,
        gross_income_usd: get_f64(conn, K_INCOME, d.gross_income_usd)?,
        years_earning: get_f64(conn, K_YEARS, d.years_earning)?,
    })
}

fn save_inputs(conn: &Connection, a: &WealthInputs) -> rusqlite::Result<()> {
    let pairs: [(&str, String); 3] = [
        (K_AGE, a.current_age.to_string()),
        (K_INCOME, a.gross_income_usd.to_string()),
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
    fn expected_and_adjusted_match_the_formulas() {
        let inputs = WealthInputs {
            current_age: 24.0,
            gross_income_usd: 215_000.0,
            years_earning: 1.16,
        };
        let b = compute_benchmark(&inputs, 93_000.0);
        assert_eq!(b.expected_usd, 516_000.0); // 24*215000/10
        assert_eq!(b.adjusted_usd, 198_461.54); // 24*215000/26
        assert!(matches!(b.status, AccumulatorStatus::Under)); // 93k/516k = 0.18
        assert!((b.velocity_usd - 80_172.41).abs() < 1.0);
    }

    #[test]
    fn prodigious_when_double_expected() {
        let inputs = WealthInputs { current_age: 50.0, gross_income_usd: 100_000.0, years_earning: 20.0 };
        // expected = 500k; 1.2M >= 1M → PAW. age 50 collapses adjusted to classic.
        let b = compute_benchmark(&inputs, 1_200_000.0);
        assert!(matches!(b.status, AccumulatorStatus::Prodigious));
        assert_eq!(b.adjusted_usd, b.expected_usd);
        assert_eq!(b.prodigious_usd, 1_000_000.0);
    }

    #[test]
    fn zero_income_is_safe() {
        let b = compute_benchmark(&WealthInputs { current_age: 30.0, gross_income_usd: 0.0, years_earning: 0.0 }, 50_000.0);
        assert_eq!(b.ratio, 0.0);
        assert_eq!(b.velocity_usd, 0.0);
        assert!(matches!(b.status, AccumulatorStatus::Under));
    }

    #[test]
    fn defaults_persist_and_reload() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), WealthInputs::default());
        let custom = WealthInputs { current_age: 24.0, gross_income_usd: 215_000.0, years_earning: 1.16 };
        save_inputs(&conn, &custom).unwrap();
        assert_eq!(load_inputs(&conn).unwrap(), custom);
    }
}
