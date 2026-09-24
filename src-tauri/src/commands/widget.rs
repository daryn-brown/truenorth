use chrono::{NaiveDate, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::OnceLock;
use tauri::State;

use super::net_worth::compute_net_worth;
use crate::{db::AppDb, fx::load_usd_rates};

const ENABLED_KEY: &str = "mac_widget_enabled";

#[derive(Debug, Serialize)]
pub struct MacWidgetSettings {
    pub platform_supported: bool,
    pub available: bool,
    pub enabled: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotState {
    Ready,
    NoAccounts,
    MissingBalances,
    MissingRates,
}

#[derive(Debug, Serialize)]
struct WidgetSnapshot {
    schema_version: u32,
    updated_at: String,
    state: SnapshotState,
    total_usd: Option<f64>,
    total_cad: Option<f64>,
    as_of: Option<String>,
}

trait SnapshotPublisher {
    fn publish(&self, snapshot: Option<&str>) -> Result<(), String>;
}

fn native_publisher() -> &'static Result<NativePublisher, String> {
    // Swift metadata and WidgetCenter callbacks must not outlive the loaded library.
    static PUBLISHER: OnceLock<Result<NativePublisher, String>> = OnceLock::new();
    PUBLISHER.get_or_init(NativePublisher::load)
}

#[tauri::command]
pub fn get_mac_widget_settings(db: State<AppDb>) -> Result<MacWidgetSettings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    settings(&conn, native_publisher())
}

#[tauri::command]
pub fn refresh_mac_widget(db: State<AppDb>) -> Result<MacWidgetSettings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let publisher = native_publisher();
    let status = settings(&conn, publisher)?;
    match publisher {
        Ok(publisher) => refresh(&conn, publisher)?,
        Err(reason) if status.enabled => return Err(reason.clone()),
        Err(_) => {}
    }
    Ok(status)
}

#[tauri::command]
pub fn set_mac_widget_enabled(
    db: State<AppDb>,
    enabled: bool,
) -> Result<MacWidgetSettings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let publisher = native_publisher();
    if !enabled {
        // Persist the opt-out before filesystem work so a failed cleanup cannot re-enable sharing.
        save_enabled(&conn, false)?;
    }
    let ready = publisher.as_ref().map_err(|reason| {
        if enabled {
            reason.clone()
        } else {
            format!("Sharing is off, but the cached snapshot could not be cleared: {reason}")
        }
    })?;
    set_enabled(&conn, enabled, ready)?;
    settings(&conn, publisher)
}

fn settings(
    conn: &Connection,
    publisher: &Result<NativePublisher, String>,
) -> Result<MacWidgetSettings, String> {
    Ok(MacWidgetSettings {
        platform_supported: cfg!(target_os = "macos"),
        available: publisher.is_ok(),
        enabled: is_enabled(conn)?,
        unavailable_reason: publisher.as_ref().err().cloned(),
    })
}

fn is_enabled(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [ENABLED_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match value.as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        Some(_) => Err("The saved widget sharing setting is invalid.".into()),
    }
}

fn save_enabled(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) \
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![ENABLED_KEY, enabled.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn set_enabled(
    conn: &Connection,
    enabled: bool,
    publisher: &impl SnapshotPublisher,
) -> Result<(), String> {
    if !enabled {
        save_enabled(conn, false)?;
        return publisher
            .publish(None)
            .map_err(|e| format!("Sharing is off, but clearing the widget failed: {e}"));
    }
    publish_current(conn, publisher)?;
    if let Err(error) = save_enabled(conn, true) {
        return Err(clear_after_error(publisher, error));
    }
    Ok(())
}

fn refresh(conn: &Connection, publisher: &impl SnapshotPublisher) -> Result<(), String> {
    if is_enabled(conn)? {
        publish_current(conn, publisher)
    } else {
        // Also remove stale exports after database recovery or an interrupted opt-out.
        publisher.publish(None)
    }
}

fn publish_current(conn: &Connection, publisher: &impl SnapshotPublisher) -> Result<(), String> {
    let result = build_snapshot(conn)
        .and_then(|snapshot| serde_json::to_string(&snapshot).map_err(|e| e.to_string()))
        .and_then(|json| publisher.publish(Some(&json)));
    result.map_err(|error| clear_after_error(publisher, error))
}

fn clear_after_error(publisher: &impl SnapshotPublisher, error: String) -> String {
    match publisher.publish(None) {
        Ok(()) => error,
        Err(cleanup) => {
            format!("{error} The previous widget snapshot could not be cleared: {cleanup}")
        }
    }
}

fn build_snapshot(conn: &Connection) -> Result<WidgetSnapshot, String> {
    let net_worth = compute_net_worth(conn)?;
    let mut snapshot = WidgetSnapshot {
        schema_version: 1,
        updated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        state: SnapshotState::NoAccounts,
        total_usd: None,
        total_cad: None,
        as_of: None,
    };
    if net_worth.accounts.is_empty() {
        return Ok(snapshot);
    }
    if net_worth
        .accounts
        .iter()
        .any(|account| account.snapshot_date.is_none())
    {
        snapshot.state = SnapshotState::MissingBalances;
        return Ok(snapshot);
    }

    let rates = load_usd_rates(conn).map_err(|e| e.to_string())?;
    let has_rate = |currency: &str| {
        rates
            .get(currency)
            .is_some_and(|rate| rate.is_finite() && *rate > 0.0)
    };
    if !has_rate("CAD")
        || net_worth
            .accounts
            .iter()
            .any(|account| !has_rate(&account.currency))
    {
        snapshot.state = SnapshotState::MissingRates;
        return Ok(snapshot);
    }
    if !net_worth.total_usd.is_finite() || !net_worth.total_cad.is_finite() {
        return Err("Net worth contains a non-finite amount; no widget totals were shared.".into());
    }

    // Use the oldest contributing balance or FX date, not the time the app was opened.
    let oldest_rate: Option<String> = conn
        .query_row(
            "SELECT MIN(f.rate_date) FROM fx_rates f \
             WHERE f.from_currency = 'USD' \
               AND (f.to_currency = 'CAD' OR f.to_currency IN \
                    (SELECT currency FROM accounts WHERE is_active = 1 AND currency != 'USD')) \
               AND f.rate_date = (SELECT MAX(rate_date) FROM fx_rates \
                    WHERE from_currency = 'USD' AND to_currency = f.to_currency)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let dates = net_worth
        .accounts
        .iter()
        .filter_map(|account| account.snapshot_date.as_deref())
        .chain(oldest_rate.as_deref());
    let mut oldest: Option<NaiveDate> = None;
    for date in dates {
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| "A balance or FX date is invalid; no widget totals were shared.")?;
        oldest = Some(oldest.map_or(parsed, |previous| previous.min(parsed)));
    }
    snapshot.state = SnapshotState::Ready;
    snapshot.total_usd = Some(net_worth.total_usd);
    snapshot.total_cad = Some(net_worth.total_cad);
    snapshot.as_of = oldest.map(|date| date.format("%Y-%m-%d").to_string());
    Ok(snapshot)
}

#[cfg(target_os = "macos")]
struct NativePublisher {
    _library: libloading::Library,
    update: unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char,
    free: unsafe extern "C" fn(*mut std::ffi::c_char),
}

#[cfg(target_os = "macos")]
impl NativePublisher {
    fn load() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let contents = executable
            .parent()
            .and_then(|path| path.parent())
            .ok_or("Cannot locate the Mac application bundle.")?;
        let library_path = contents.join("Frameworks/libTrueNorthWidgetBridge.dylib");
        let extension = contents.join("PlugIns/TrueNorthWidget.appex");
        if !library_path.is_file() || !extension.is_dir() {
            return Err(
                "Widgets require the signed macOS widget build, installed and opened as an app. \
                 They are not included in unsigned builds or the development server."
                    .into(),
            );
        }
        // Only load the bridge embedded in our signed app, never a frontend-supplied library.
        unsafe {
            let library = libloading::Library::new(&library_path)
                .map_err(|e| format!("The widget bridge requires macOS 14 or later: {e}"))?;
            let check = *library
                .get::<unsafe extern "C" fn() -> *mut std::ffi::c_char>(b"truenorth_widget_check\0")
                .map_err(|e| e.to_string())?;
            let publisher = Self {
                update: *library
                    .get(b"truenorth_widget_update\0")
                    .map_err(|e| e.to_string())?,
                free: *library
                    .get(b"truenorth_widget_free\0")
                    .map_err(|e| e.to_string())?,
                _library: library,
            };
            publisher.check_result(check())?;
            Ok(publisher)
        }
    }

    unsafe fn check_result(&self, error: *mut std::ffi::c_char) -> Result<(), String> {
        if error.is_null() {
            return Ok(());
        }
        let message = std::ffi::CStr::from_ptr(error)
            .to_string_lossy()
            .into_owned();
        (self.free)(error);
        Err(message)
    }
}

#[cfg(target_os = "macos")]
impl SnapshotPublisher for NativePublisher {
    fn publish(&self, snapshot: Option<&str>) -> Result<(), String> {
        let json = snapshot
            .map(std::ffi::CString::new)
            .transpose()
            .map_err(|e| e.to_string())?;
        unsafe {
            self.check_result((self.update)(
                json.as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
            ))
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct NativePublisher;

#[cfg(not(target_os = "macos"))]
impl NativePublisher {
    fn load() -> Result<Self, String> {
        Err("Desktop widgets are available only on macOS 14 or later.".into())
    }
}

#[cfg(not(target_os = "macos"))]
impl SnapshotPublisher for NativePublisher {
    fn publish(&self, _: Option<&str>) -> Result<(), String> {
        Err("Desktop widgets are available only on macOS 14 or later.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Publisher {
        snapshot: RefCell<Option<String>>,
        fail: bool,
    }

    impl SnapshotPublisher for Publisher {
        fn publish(&self, snapshot: Option<&str>) -> Result<(), String> {
            if self.fail {
                return Err("Shared container is unavailable.".into());
            }
            *self.snapshot.borrow_mut() = snapshot.map(str::to_string);
            Ok(())
        }
    }

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        crate::fx::store_usd_rate(&conn, "CAD", 1.25, "2026-09-24").unwrap();
        conn
    }

    fn account(conn: &Connection, currency: &str, balance: Option<f64>, date: &str) {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction) \
             VALUES ('Private account name', 'Private institution', 'savings', ?1, 'US')",
            [currency],
        )
        .unwrap();
        if let Some(balance) = balance {
            conn.execute(
                "INSERT INTO balance_snapshots (account_id, snapshot_date, balance, currency) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![conn.last_insert_rowid(), date, balance, currency],
            )
            .unwrap();
        }
    }

    #[test]
    fn shares_only_aggregates_and_the_oldest_contributing_date() {
        let conn = setup();
        crate::fx::store_usd_rate(&conn, "EUR", 0.8, "2026-09-20").unwrap();
        account(&conn, "USD", Some(100.0), "2026-09-23");
        account(&conn, "CAD", Some(-25.0), "2026-09-24");
        account(&conn, "EUR", Some(80.0), "2026-09-24");
        let snapshot = build_snapshot(&conn).unwrap();
        assert_eq!(snapshot.state, SnapshotState::Ready);
        assert_eq!(snapshot.total_usd, Some(180.0));
        assert_eq!(snapshot.total_cad, Some(225.0));
        assert_eq!(snapshot.as_of.as_deref(), Some("2026-09-20"));
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 6);
        assert!(!json.to_string().contains("Private"));
        assert!(!json.to_string().contains("accounts"));
    }

    #[test]
    fn missing_and_invalid_rates_never_look_like_zero_net_worth() {
        for rate in [None, Some(0.0), Some(-1.0), Some(f64::INFINITY)] {
            let conn = setup();
            account(&conn, "EUR", Some(10.0), "2026-09-24");
            if let Some(rate) = rate {
                crate::fx::store_usd_rate(&conn, "EUR", rate, "2026-09-24").unwrap();
            }
            let snapshot = build_snapshot(&conn).unwrap();
            assert_eq!(snapshot.state, SnapshotState::MissingRates);
            assert!(snapshot.total_usd.is_none());
            assert!(snapshot.total_cad.is_none());
        }
    }

    #[test]
    fn no_accounts_and_missing_balances_have_distinct_empty_states() {
        let conn = setup();
        assert_eq!(
            build_snapshot(&conn).unwrap().state,
            SnapshotState::NoAccounts
        );
        account(&conn, "USD", None, "2026-09-24");
        let snapshot = build_snapshot(&conn).unwrap();
        assert_eq!(snapshot.state, SnapshotState::MissingBalances);
        assert!(snapshot.total_usd.is_none());
    }

    #[test]
    fn sharing_is_opt_in_and_disabling_removes_the_snapshot() {
        let conn = setup();
        account(&conn, "USD", Some(100.0), "2026-09-24");
        let publisher = Publisher::default();
        assert!(!is_enabled(&conn).unwrap());
        refresh(&conn, &publisher).unwrap();
        assert!(publisher.snapshot.borrow().is_none());
        set_enabled(&conn, true, &publisher).unwrap();
        assert!(is_enabled(&conn).unwrap());
        assert!(publisher.snapshot.borrow().is_some());
        set_enabled(&conn, false, &publisher).unwrap();
        assert!(!is_enabled(&conn).unwrap());
        assert!(publisher.snapshot.borrow().is_none());
        refresh(&conn, &publisher).unwrap();
        assert!(publisher.snapshot.borrow().is_none());
    }

    #[test]
    fn deleting_accounts_does_not_leave_old_totals() {
        let conn = setup();
        account(&conn, "USD", Some(100.0), "2026-09-24");
        let publisher = Publisher::default();
        set_enabled(&conn, true, &publisher).unwrap();
        conn.execute("UPDATE accounts SET is_active = 0", [])
            .unwrap();
        refresh(&conn, &publisher).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(publisher.snapshot.borrow().as_ref().unwrap()).unwrap();
        assert_eq!(json["state"], "no_accounts");
        assert!(json["total_usd"].is_null());
    }

    #[test]
    fn failed_cleanup_stays_opted_out_and_reports_the_error() {
        let conn = setup();
        save_enabled(&conn, true).unwrap();
        let publisher = Publisher {
            fail: true,
            ..Default::default()
        };
        assert!(set_enabled(&conn, false, &publisher)
            .unwrap_err()
            .contains("clearing"));
        assert!(!is_enabled(&conn).unwrap());
        assert!(set_enabled(&conn, true, &publisher).is_err());
        assert!(!is_enabled(&conn).unwrap());
    }

    #[test]
    fn corrupt_preferences_are_not_treated_as_consent() {
        let conn = setup();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, 'yes')",
            [ENABLED_KEY],
        )
        .unwrap();
        assert!(is_enabled(&conn).is_err());
    }

    #[test]
    fn failure_to_persist_consent_removes_the_export() {
        let conn = setup();
        account(&conn, "USD", Some(100.0), "2026-09-24");
        conn.execute_batch(
            "CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings \
             BEGIN SELECT RAISE(ABORT, 'Cannot save settings'); END;",
        )
        .unwrap();
        let publisher = Publisher::default();
        assert!(set_enabled(&conn, true, &publisher).is_err());
        assert!(!is_enabled(&conn).unwrap());
        assert!(publisher.snapshot.borrow().is_none());
    }

    #[test]
    fn both_totals_require_the_cad_rate() {
        let conn = setup();
        account(&conn, "USD", Some(100.0), "2026-09-24");
        conn.execute("DELETE FROM fx_rates", []).unwrap();
        let snapshot = build_snapshot(&conn).unwrap();
        assert_eq!(snapshot.state, SnapshotState::MissingRates);
        assert!(snapshot.total_usd.is_none());
        assert!(snapshot.total_cad.is_none());
    }

    #[test]
    fn invalid_data_clears_a_previously_shared_snapshot() {
        let conn = setup();
        account(&conn, "USD", Some(100.0), "2026-09-24");
        let publisher = Publisher::default();
        set_enabled(&conn, true, &publisher).unwrap();
        conn.execute("UPDATE balance_snapshots SET snapshot_date = 'invalid'", [])
            .unwrap();
        assert!(refresh(&conn, &publisher).is_err());
        assert!(publisher.snapshot.borrow().is_none());
    }
}
