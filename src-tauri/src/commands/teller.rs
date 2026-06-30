//! Teller Tauri commands: save the certificate/config, add an enrollment from Teller Connect, sync
//! balances, and disconnect.
//!
//! Teller is a free-for-personal-use US bank aggregator. Unlike SimpleFIN's single access URL, a
//! Teller integration has three pieces of state:
//!
//! * a **client certificate** (cert + private key PEM) used for mTLS — stored in the secret store;
//! * one or more **access tokens** (one per Teller Connect enrollment) — stored as a JSON array in
//!   the secret store;
//! * non-secret **config** (application id + environment) — stored in `app_settings`.
//!
//! As with the other connectors, the SQLite mutex is never held across an `.await`: every network
//! call happens first, then the results are written under a short-lived lock, and one balance
//! snapshot per account flows straight into the existing net-worth pipeline. Teller reports a credit
//! card's balance as a positive "amount owed", so credit accounts are stored with a negative sign to
//! subtract from net worth.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::connector::teller::{TellerAccount, TellerClient, TellerError};
use crate::db::secrets;
use crate::db::AppDb;

const SETTING_ENVIRONMENT: &str = "teller_environment";
const SETTING_APPLICATION_ID: &str = "teller_application_id";
const SETTING_LAST_SYNCED: &str = "teller_last_synced_at";

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct TellerStatus {
    /// At least one enrollment (access token) is stored.
    pub is_connected: bool,
    /// A client certificate + private key are stored (required for development/production).
    pub has_certificate: bool,
    /// `sandbox` (default), `development`, or `production`.
    pub environment: String,
    /// Public Teller application id (`app_…`), for display and for driving Teller Connect.
    pub application_id: Option<String>,
    /// How many enrollments (institution logins) are stored.
    pub enrollment_count: usize,
    /// Number of active accounts connected via Teller.
    pub account_count: i64,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TellerSyncSummary {
    pub accounts_synced: usize,
    pub synced_at: String,
    /// Non-fatal messages (e.g. one enrollment needs re-authentication).
    pub warnings: Vec<String>,
}

/// One stored Teller enrollment. Persisted as JSON in the secret store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TellerEnrollment {
    access_token: String,
    #[serde(default)]
    institution: Option<String>,
    #[serde(default)]
    enrollment_id: Option<String>,
}

// ---------------------------------------------------------------------------
// app_settings helpers
// ---------------------------------------------------------------------------

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) \
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value],
    )?;
    Ok(())
}

fn delete_setting(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Secret-store helpers (certificate + enrollments)
// ---------------------------------------------------------------------------

/// Load the stored enrollments, or an empty list when none are saved.
fn load_enrollments() -> Result<Vec<TellerEnrollment>, String> {
    match secrets::get_secret(secrets::TELLER_ENROLLMENTS).map_err(|e| e.to_string())? {
        Some(json) => serde_json::from_str(&json)
            .map_err(|e| format!("Stored Teller enrollments are corrupt: {e}")),
        None => Ok(Vec::new()),
    }
}

fn save_enrollments(list: &[TellerEnrollment]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    secrets::set_secret(secrets::TELLER_ENROLLMENTS, &json).map_err(|e| e.to_string())
}

/// Build the combined PEM identity bytes (certificate + private key) when both are stored.
fn load_identity_pem() -> Result<Option<Vec<u8>>, String> {
    let cert = secrets::get_secret(secrets::TELLER_CERT_PEM).map_err(|e| e.to_string())?;
    let key = secrets::get_secret(secrets::TELLER_KEY_PEM).map_err(|e| e.to_string())?;
    match (cert, key) {
        (Some(c), Some(k)) => Ok(Some(format!("{c}\n{k}").into_bytes())),
        _ => Ok(None),
    }
}

fn read_environment(conn: &Connection) -> rusqlite::Result<String> {
    Ok(get_setting(conn, SETTING_ENVIRONMENT)?.unwrap_or_else(|| "sandbox".to_string()))
}

fn environment_from_db(db: &State<AppDb>) -> Result<String, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    read_environment(&conn).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

fn normalize_env(env: &str) -> String {
    match env.trim().to_lowercase().as_str() {
        "development" => "development",
        "production" => "production",
        _ => "sandbox",
    }
    .to_string()
}

/// Map Teller's `type`/`subtype` (with a name fallback) to one of TrueNorth's account-type ids.
fn map_account_type(kind: &str, subtype: Option<&str>, name: &str) -> String {
    if let Some(st) = subtype {
        match st {
            "checking" => return "chequing".to_string(),
            "savings" | "money_market" | "certificate_of_deposit" | "treasury" | "sweep" => {
                return "savings".to_string()
            }
            "credit_card" => return "credit".to_string(),
            _ => {}
        }
    }
    if kind.eq_ignore_ascii_case("credit") {
        return "credit".to_string();
    }
    let hay = name.to_uppercase();
    if hay.contains("SAVING") {
        return "savings".to_string();
    }
    if hay.contains("CREDIT") || hay.contains("CARD") || hay.contains("VISA") || hay.contains("MASTERCARD") {
        return "credit".to_string();
    }
    // Teller is deposit-focused; default a plain account to chequing.
    "chequing".to_string()
}

/// Net worth sums signed balances. Teller reports a credit card's owed amount as a *positive*
/// number, so liabilities are negated to subtract; deposit balances are kept as reported.
fn signed_balance(account_type: &str, raw: f64) -> f64 {
    if account_type == "credit" {
        -raw.abs()
    } else {
        raw
    }
}

/// Turn a Teller error into a user-facing message.
fn friendly(e: TellerError) -> String {
    if e.is_auth() {
        "Teller rejected the request. Re-link the bank with Teller Connect and confirm your client \
         certificate belongs to the same Teller application."
            .into()
    } else {
        e.to_string()
    }
}

fn enrollment_label(enr: &TellerEnrollment) -> String {
    enr.institution
        .clone()
        .or_else(|| enr.enrollment_id.clone())
        .unwrap_or_else(|| "Teller enrollment".to_string())
}

// ---------------------------------------------------------------------------
// DB reconcile (synchronous — never run while awaiting)
// ---------------------------------------------------------------------------

/// Upsert one Teller account (keyed by `connector_ref`) and write today's balance snapshot. Returns
/// the local account row id.
fn upsert_account(
    conn: &Connection,
    account: &TellerAccount,
    balance: Option<f64>,
    today: &str,
    now: &str,
) -> rusqlite::Result<i64> {
    let account_type = map_account_type(&account.kind, account.subtype.as_deref(), &account.name);
    let reported_currency = if account.currency.is_empty() {
        "USD"
    } else {
        account.currency.as_str()
    };
    let institution = account
        .institution
        .clone()
        .unwrap_or_else(|| "Teller".to_string());

    // Keyed by connector_ref. As with SimpleFIN, an existing account keeps its stored currency so a
    // user's manual currency correction survives re-syncs.
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, currency FROM accounts WHERE connector_kind = 'teller' AND connector_ref = ?1",
            params![account.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let (account_id, currency) = if let Some((id, stored_currency)) = existing {
        conn.execute(
            "UPDATE accounts SET name = ?1, institution = ?2, account_type = ?3, \
             is_active = 1, updated_at = ?4 WHERE id = ?5",
            params![account.name, institution, account_type, now, id],
        )?;
        (id, stored_currency)
    } else {
        conn.execute(
            "INSERT INTO accounts \
             (name, institution, account_type, currency, jurisdiction, connector_kind, connector_ref) \
             VALUES (?1, ?2, ?3, ?4, 'US', 'teller', ?5)",
            params![account.name, institution, account_type, reported_currency, account.id],
        )?;
        (conn.last_insert_rowid(), reported_currency.to_string())
    };

    if let Some(raw) = balance {
        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, ?2, ?3, ?4, 'teller')",
            params![account_id, today, signed_balance(&account_type, raw), currency],
        )?;
    }

    Ok(account_id)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Report whether Teller is connected, plus environment/certificate/account details.
#[tauri::command]
pub fn teller_get_status(db: State<AppDb>) -> Result<TellerStatus, String> {
    let (last_synced_at, account_count, environment, application_id) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let last_synced_at = get_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        let account_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'teller' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let environment = read_environment(&conn).map_err(|e| e.to_string())?;
        let application_id =
            get_setting(&conn, SETTING_APPLICATION_ID).map_err(|e| e.to_string())?;
        (last_synced_at, account_count, environment, application_id)
    };

    let enrollments = load_enrollments()?;
    let has_certificate = load_identity_pem()?.is_some();

    Ok(TellerStatus {
        is_connected: !enrollments.is_empty(),
        has_certificate,
        environment,
        application_id,
        enrollment_count: enrollments.len(),
        account_count,
        last_synced_at,
    })
}

/// Save the Teller application id, environment, and (optionally) the client certificate + key.
/// Provide both the certificate and key, or neither. The development/production environments
/// require a certificate.
#[tauri::command]
pub fn teller_save_config(
    db: State<AppDb>,
    application_id: String,
    environment: String,
    certificate: Option<String>,
    private_key: Option<String>,
) -> Result<TellerStatus, String> {
    let application_id = application_id.trim().to_string();
    if application_id.is_empty() {
        return Err("Enter your Teller application id (it starts with \"app_\").".into());
    }
    let environment = normalize_env(&environment);

    let cert = certificate
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let key = private_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());

    match (&cert, &key) {
        (Some(c), Some(k)) => {
            // Validate the pair parses as a TLS identity before storing it.
            let pem = format!("{c}\n{k}");
            reqwest::Identity::from_pem(pem.as_bytes()).map_err(|e| {
                format!("That certificate or private key couldn't be read as PEM: {e}")
            })?;
            secrets::set_secret(secrets::TELLER_CERT_PEM, c).map_err(|e| e.to_string())?;
            secrets::set_secret(secrets::TELLER_KEY_PEM, k).map_err(|e| e.to_string())?;
        }
        (None, None) => {}
        _ => {
            return Err(
                "Provide both the client certificate and the private key, or leave both blank.".into(),
            )
        }
    }

    if environment != "sandbox" && load_identity_pem()?.is_none() {
        return Err("The development and production environments need a Teller client \
                    certificate. Paste your certificate and private key, or switch to the sandbox \
                    environment."
            .into());
    }

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_setting(&conn, SETTING_APPLICATION_ID, &application_id).map_err(|e| e.to_string())?;
        set_setting(&conn, SETTING_ENVIRONMENT, &environment).map_err(|e| e.to_string())?;
    }

    teller_get_status(db)
}

/// Store an access token returned by Teller Connect. The token is verified by listing accounts
/// (which also captures the institution name), so a dead token is never saved.
#[tauri::command]
pub async fn teller_add_enrollment(
    db: State<'_, AppDb>,
    access_token: String,
    institution: Option<String>,
    enrollment_id: Option<String>,
) -> Result<TellerStatus, String> {
    let access_token = access_token.trim().to_string();
    if access_token.is_empty() {
        return Err("No Teller access token was provided.".into());
    }

    let environment = environment_from_db(&db)?;
    let identity = load_identity_pem()?;
    if environment != "sandbox" && identity.is_none() {
        return Err("Save your Teller client certificate before linking a bank.".into());
    }

    // Verify the token over the network (no DB lock held).
    let client = TellerClient::new(&access_token, identity.as_deref()).map_err(friendly)?;
    let accounts = client.check().await.map_err(friendly)?;

    let institution =
        institution.or_else(|| accounts.iter().find_map(|a| a.institution.clone()));
    let enrollment_id =
        enrollment_id.or_else(|| accounts.iter().find_map(|a| a.enrollment_id.clone()));

    let mut enrollments = load_enrollments()?;
    // De-dupe: replace a prior enrollment for the same institution login (by enrollment id when we
    // have one, else by the token itself).
    enrollments.retain(|e| match (&enrollment_id, &e.enrollment_id) {
        (Some(new), Some(old)) => new != old,
        _ => e.access_token != access_token,
    });
    enrollments.push(TellerEnrollment {
        access_token,
        institution,
        enrollment_id,
    });
    save_enrollments(&enrollments)?;

    teller_get_status(db)
}

/// Pull accounts + live balances from every Teller enrollment and reconcile them into the local DB.
/// Writes one balance snapshot per account so net worth updates automatically.
#[tauri::command]
pub async fn teller_sync(db: State<'_, AppDb>) -> Result<TellerSyncSummary, String> {
    let enrollments = load_enrollments()?;
    if enrollments.is_empty() {
        return Err("Link a bank with Teller before syncing.".into());
    }

    let environment = environment_from_db(&db)?;
    let identity = load_identity_pem()?;
    if environment != "sandbox" && identity.is_none() {
        return Err("Save your Teller client certificate before syncing.".into());
    }

    // Fetch everything over the network first — no DB lock is held across an await.
    let mut fetched: Vec<(TellerAccount, Option<f64>)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for enr in &enrollments {
        let client = match TellerClient::new(&enr.access_token, identity.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("{}: {}", enrollment_label(enr), friendly(e)));
                continue;
            }
        };
        let accounts = match client.fetch_accounts().await {
            Ok(a) => a,
            Err(e) => {
                warnings.push(format!("{}: {}", enrollment_label(enr), friendly(e)));
                continue;
            }
        };
        for account in accounts {
            if account.status.as_deref() == Some("closed") {
                continue;
            }
            let balance = match client.fetch_balance(&account.id).await {
                Ok(b) => b.primary(),
                Err(e) => {
                    warnings.push(format!("{}: {}", account.name, friendly(e)));
                    None
                }
            };
            fetched.push((account, balance));
        }
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut accounts_synced = 0usize;
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (account, balance) in &fetched {
            upsert_account(&tx, account, *balance, &today, &now).map_err(|e| e.to_string())?;
            accounts_synced += 1;
        }
        set_setting(&tx, SETTING_LAST_SYNCED, &now).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(TellerSyncSummary {
        accounts_synced,
        synced_at: now,
        warnings,
    })
}

/// Disconnect Teller: drop the stored enrollments + client certificate, clear the last-synced
/// marker, and deactivate the connected accounts. The application id/environment config and
/// historical snapshots are kept.
#[tauri::command]
pub fn teller_disconnect(db: State<AppDb>) -> Result<TellerStatus, String> {
    secrets::delete_secret(secrets::TELLER_ENROLLMENTS).map_err(|e| e.to_string())?;
    secrets::delete_secret(secrets::TELLER_CERT_PEM).map_err(|e| e.to_string())?;
    secrets::delete_secret(secrets::TELLER_KEY_PEM).map_err(|e| e.to_string())?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        delete_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE connector_kind = 'teller'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    teller_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_account_types_from_subtype_then_kind_then_name() {
        assert_eq!(map_account_type("depository", Some("checking"), "x"), "chequing");
        assert_eq!(map_account_type("depository", Some("savings"), "x"), "savings");
        assert_eq!(map_account_type("depository", Some("money_market"), "x"), "savings");
        assert_eq!(map_account_type("credit", Some("credit_card"), "x"), "credit");
        // No subtype: fall back to the account class.
        assert_eq!(map_account_type("credit", None, "Mystery"), "credit");
        // No subtype, depository: name heuristics, else chequing.
        assert_eq!(map_account_type("depository", None, "Online Savings"), "savings");
        assert_eq!(map_account_type("depository", None, "Platinum Card"), "credit");
        assert_eq!(map_account_type("depository", None, "Everyday"), "chequing");
    }

    #[test]
    fn credit_balances_are_stored_negative() {
        // Teller reports an owed credit-card balance as positive; it must subtract from net worth.
        assert_eq!(signed_balance("credit", 500.0), -500.0);
        assert_eq!(signed_balance("credit", -500.0), -500.0);
        // Deposit balances pass through unchanged (including a genuine overdraft).
        assert_eq!(signed_balance("chequing", 1000.0), 1000.0);
        assert_eq!(signed_balance("chequing", -25.0), -25.0);
    }

    #[test]
    fn normalize_env_defaults_to_sandbox() {
        assert_eq!(normalize_env("Development"), "development");
        assert_eq!(normalize_env(" production "), "production");
        assert_eq!(normalize_env("anything-else"), "sandbox");
        assert_eq!(normalize_env(""), "sandbox");
    }
}
