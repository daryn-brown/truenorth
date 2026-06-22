//! Goal tracking — the "$100k Countdown" / CoastFIRE projection.
//!
//! The headline milestone is a single net-worth target in **USD** (the benchmark currency),
//! stored in `app_settings` and seeded to $100,000. `get_goal_progress` reuses the net-worth
//! history series to report how far along the user is and, from their **rolling ~30-day pace**
//! (net-worth change per day), projects the date they'll cross the target. The pace is
//! balance-driven (history uses a single FX rate), so it reflects real saving + growth rather
//! than FX noise.

use chrono::{Duration, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::commands::net_worth::{compute_net_worth_history, NetWorthHistoryPoint};
use crate::db::AppDb;

const SETTING_GOAL_TARGET: &str = "goal_target_usd";
const DEFAULT_TARGET_USD: f64 = 100_000.0;
const ROLLING_WINDOW_DAYS: i64 = 30;

#[derive(Debug, Serialize, PartialEq)]
pub struct GoalProgress {
    /// The milestone, in USD.
    pub target_usd: f64,
    /// Current net worth, in USD.
    pub current_usd: f64,
    /// Remaining distance to the target (0 once met).
    pub gap_usd: f64,
    /// Fraction complete, clamped to 0..1.
    pub progress: f64,
    pub already_met: bool,
    /// Net-worth change per day over the trailing window (the "savings rate" proxy). None until
    /// there are at least two snapshot dates.
    pub daily_rate_usd: Option<f64>,
    /// That same pace expressed per 30 days, for display.
    pub monthly_rate_usd: Option<f64>,
    /// Actual span (days) the pace was measured over — may exceed 30 when history is sparse.
    pub window_days: Option<i64>,
    /// Projected date (YYYY-MM-DD) net worth reaches the target at the current pace. None when the
    /// goal is met, the pace is flat/negative, or there isn't enough history to project.
    pub projected_date: Option<String>,
    /// Days from the latest snapshot to `projected_date` (0 when already met).
    pub days_to_goal: Option<i64>,
}

/// Current progress toward the net-worth milestone plus a projected hit-date.
#[tauri::command]
pub fn get_goal_progress(db: State<AppDb>) -> Result<GoalProgress, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let target = get_target_usd(&conn).map_err(|e| e.to_string())?;
    let history = compute_net_worth_history(&conn).map_err(|e| e.to_string())?;
    Ok(compute_goal_progress(target, &history))
}

/// Update the milestone target (USD) and return the recomputed progress.
#[tauri::command]
pub fn set_goal_target(db: State<AppDb>, target_usd: f64) -> Result<GoalProgress, String> {
    if !target_usd.is_finite() || target_usd <= 0.0 {
        return Err("Goal target must be a positive amount.".into());
    }
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) \
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![SETTING_GOAL_TARGET, target_usd.to_string()],
    )
    .map_err(|e| e.to_string())?;
    let history = compute_net_worth_history(&conn).map_err(|e| e.to_string())?;
    Ok(compute_goal_progress(target_usd, &history))
}

fn get_target_usd(conn: &Connection) -> rusqlite::Result<f64> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_GOAL_TARGET],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(DEFAULT_TARGET_USD))
}

fn compute_goal_progress(target_usd: f64, history: &[NetWorthHistoryPoint]) -> GoalProgress {
    let current_usd = history.last().map(|p| p.total_usd).unwrap_or(0.0);
    let gap_usd = (target_usd - current_usd).max(0.0);
    let progress = if target_usd > 0.0 {
        (current_usd / target_usd).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let already_met = current_usd >= target_usd;

    let (daily_rate_usd, window_days) = match rolling_daily_rate(history) {
        Some((rate, days)) => (Some(rate), Some(days)),
        None => (None, None),
    };
    let monthly_rate_usd = daily_rate_usd.map(|r| r * ROLLING_WINDOW_DAYS as f64);

    let (projected_date, days_to_goal) = if already_met {
        (None, Some(0))
    } else {
        match (daily_rate_usd, history.last()) {
            (Some(rate), Some(last)) if rate > 0.0 => {
                let days = (gap_usd / rate).ceil() as i64;
                let date = NaiveDate::parse_from_str(&last.date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.checked_add_signed(Duration::days(days)))
                    .map(|d| d.format("%Y-%m-%d").to_string());
                (date, Some(days))
            }
            _ => (None, None),
        }
    };

    GoalProgress {
        target_usd,
        current_usd,
        gap_usd,
        progress,
        already_met,
        daily_rate_usd,
        monthly_rate_usd,
        window_days,
        projected_date,
        days_to_goal,
    }
}

/// The net-worth change per day over roughly the last [`ROLLING_WINDOW_DAYS`]. Picks, among all
/// points before the latest, the one whose date is closest to "30 days ago" as the anchor, then
/// divides the USD change by the actual span. Returns `(rate_per_day, span_days)`.
fn rolling_daily_rate(history: &[NetWorthHistoryPoint]) -> Option<(f64, i64)> {
    if history.len() < 2 {
        return None;
    }
    let last = history.last()?;
    let last_date = NaiveDate::parse_from_str(&last.date, "%Y-%m-%d").ok()?;
    let window_start = last_date - Duration::days(ROLLING_WINDOW_DAYS);

    let (anchor_point, anchor_date) = history[..history.len() - 1]
        .iter()
        .filter_map(|p| {
            NaiveDate::parse_from_str(&p.date, "%Y-%m-%d")
                .ok()
                .map(|d| (p, d))
        })
        .min_by_key(|(_, d)| (*d - window_start).num_days().abs())?;

    let span = (last_date - anchor_date).num_days();
    if span <= 0 {
        return None;
    }
    Some((
        (last.total_usd - anchor_point.total_usd) / span as f64,
        span,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::apply_schema;
    use crate::db::seed_defaults;

    fn point(date: &str, total_usd: f64) -> NetWorthHistoryPoint {
        NetWorthHistoryPoint {
            date: date.to_string(),
            total_usd,
            total_cad: total_usd * 1.3,
        }
    }

    #[test]
    fn projects_a_hit_date_from_the_rolling_pace() {
        // +$6,000 over the 30 days before the latest point → $200/day.
        let history = vec![point("2026-01-01", 64_000.0), point("2026-01-31", 70_000.0)];
        let g = compute_goal_progress(100_000.0, &history);

        assert!(!g.already_met);
        assert_eq!(g.current_usd, 70_000.0);
        assert_eq!(g.gap_usd, 30_000.0);
        assert!((g.progress - 0.7).abs() < 1e-9);
        assert_eq!(g.daily_rate_usd, Some(200.0));
        assert_eq!(g.monthly_rate_usd, Some(6_000.0));
        assert_eq!(g.window_days, Some(30));
        // 30_000 / 200 = 150 days past 2026-01-31.
        assert_eq!(g.days_to_goal, Some(150));
        assert_eq!(g.projected_date.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn no_eta_when_pace_is_flat_or_negative() {
        let history = vec![point("2026-01-01", 70_000.0), point("2026-01-31", 68_000.0)];
        let g = compute_goal_progress(100_000.0, &history);
        assert_eq!(g.daily_rate_usd, Some(-2_000.0 / 30.0));
        assert_eq!(g.projected_date, None);
        assert_eq!(g.days_to_goal, None);
        // Progress + gap are still reported so the bar renders.
        assert_eq!(g.gap_usd, 32_000.0);
    }

    #[test]
    fn already_met_reports_full_progress() {
        let history = vec![
            point("2026-01-01", 99_000.0),
            point("2026-02-01", 101_000.0),
        ];
        let g = compute_goal_progress(100_000.0, &history);
        assert!(g.already_met);
        assert_eq!(g.gap_usd, 0.0);
        assert_eq!(g.progress, 1.0);
        assert_eq!(g.days_to_goal, Some(0));
        assert_eq!(g.projected_date, None);
    }

    #[test]
    fn single_point_has_no_pace_but_still_has_progress() {
        let history = vec![point("2026-01-01", 50_000.0)];
        let g = compute_goal_progress(100_000.0, &history);
        assert_eq!(g.daily_rate_usd, None);
        assert_eq!(g.window_days, None);
        assert_eq!(g.projected_date, None);
        assert!((g.progress - 0.5).abs() < 1e-9);
        assert_eq!(g.gap_usd, 50_000.0);
    }

    #[test]
    fn empty_history_is_zeroed() {
        let g = compute_goal_progress(100_000.0, &[]);
        assert_eq!(g.current_usd, 0.0);
        assert_eq!(g.gap_usd, 100_000.0);
        assert_eq!(g.progress, 0.0);
        assert!(!g.already_met);
        assert_eq!(g.projected_date, None);
    }

    #[test]
    fn target_defaults_to_100k_and_reads_overrides() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        seed_defaults(&conn).unwrap();

        assert_eq!(get_target_usd(&conn).unwrap(), 100_000.0);

        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('goal_target_usd', '250000') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .unwrap();
        assert_eq!(get_target_usd(&conn).unwrap(), 250_000.0);

        // A garbage override falls back to the default rather than breaking the card.
        conn.execute(
            "UPDATE app_settings SET value = 'oops' WHERE key = 'goal_target_usd'",
            [],
        )
        .unwrap();
        assert_eq!(get_target_usd(&conn).unwrap(), 100_000.0);
    }
}
