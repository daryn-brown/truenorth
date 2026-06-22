//! SnapTrade Tauri commands: credential management, the connection-portal flow, and the
//! sync that pulls real balances + holdings into the local schema.
//!
//! Async commands never hold the SQLite mutex across an `.await` (the guard is not `Send`):
//! all network calls happen first, then results are written under a short-lived lock.
//!
//! Because net worth and its history chart are derived from the latest `balance_snapshots`
//! per account, writing one snapshot per connected account during sync makes real balances
//! flow into the existing dashboard with no changes to the net-worth pipeline.

use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::connector::snaptrade::{SnapAccount, SnapPosition, SnapTradeClient, SnapTradeError};
use crate::db::secrets::{self, SNAPTRADE_CONSUMER_KEY, SNAPTRADE_USER_SECRET};
use crate::db::AppDb;

/// Non-secret identifiers live in `app_settings`; secrets live in the OS keychain.
const SETTING_CLIENT_ID: &str = "snaptrade_client_id";
const SETTING_USER_ID: &str = "snaptrade_user_id";
const SETTING_LAST_SYNCED: &str = "snaptrade_last_synced_at";

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SnapTradeStatus {
    /// API key pair is saved (clientId in settings + consumerKey in keychain).
    pub has_credentials: bool,
    /// A SnapTrade user exists (userId in settings + userSecret in keychain).
    pub is_connected: bool,
    /// The public clientId, for display. Never includes the secret consumerKey.
    pub client_id: Option<String>,
    pub last_synced_at: Option<String>,
    /// Number of active accounts connected via SnapTrade.
    pub account_count: i64,
}

#[derive(Debug, Serialize)]
pub struct SnapTradeSyncSummary {
    pub accounts_synced: usize,
    pub holdings_synced: usize,
    pub synced_at: String,
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
// Mapping helpers
// ---------------------------------------------------------------------------

/// Best-effort map from a brokerage's free-form account type / name to one of TrueNorth's
/// account-type ids. Registered-account keywords win over the generic "brokerage" fallback.
fn map_account_type(raw_type: Option<&str>, name: Option<&str>) -> String {
    let hay = format!("{} {}", raw_type.unwrap_or(""), name.unwrap_or("")).to_uppercase();
    let has = |needle: &str| hay.contains(needle);

    let kind = if has("ROTH") {
        "roth_ira"
    } else if has("401") {
        "401k"
    } else if has("RRSP") {
        "rrsp"
    } else if has("TFSA") {
        "tfsa"
    } else if has("FHSA") {
        "fhsa"
    } else if has("IRA") {
        "ira"
    } else if has("CHEQUING") || has("CHECKING") {
        "chequing"
    } else if has("SAVING") {
        "savings"
    } else if has("CREDIT") {
        "credit"
    } else if has("CRYPTO") {
        "crypto"
    } else {
        "brokerage"
    };
    kind.to_string()
}

/// SnapTrade reports balances in the account's native currency; we map that to the
/// jurisdiction the rest of the app reasons about.
fn jurisdiction_for(currency: &str) -> &'static str {
    if currency.eq_ignore_ascii_case("CAD") {
        "CA"
    } else {
        "US"
    }
}

/// Turn a SnapTrade API error into a user-facing message.
fn friendly(e: SnapTradeError) -> String {
    if e.is_auth() {
        "SnapTrade rejected the credentials. Double-check your Client ID and Consumer Key.".into()
    } else {
        e.to_string()
    }
}

/// Generate a fresh, immutable SnapTrade user id for this installation.
fn generate_user_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("truenorth-{}", hex::encode(bytes))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Report whether credentials are saved, whether a brokerage is connected, and basic stats.
#[tauri::command]
pub fn snaptrade_get_status(db: State<AppDb>) -> Result<SnapTradeStatus, String> {
    let (client_id, user_id, last_synced_at, account_count) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let client_id = get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?;
        let user_id = get_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?;
        let last_synced_at = get_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        let account_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE connector_kind = 'snaptrade' AND is_active = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        (client_id, user_id, last_synced_at, account_count)
    };

    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY).map_err(|e| e.to_string())?;
    let user_secret = secrets::get_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;

    Ok(SnapTradeStatus {
        has_credentials: client_id.is_some() && consumer_key.is_some(),
        is_connected: user_id.is_some() && user_secret.is_some(),
        client_id,
        last_synced_at,
        account_count,
    })
}

/// Validate and persist the SnapTrade API key pair. The `consumerKey` goes to the OS keychain;
/// the `clientId` to `app_settings`. Validation hits SnapTrade before anything is saved.
#[tauri::command]
pub async fn snaptrade_save_credentials(
    db: State<'_, AppDb>,
    client_id: String,
    consumer_key: String,
) -> Result<SnapTradeStatus, String> {
    let client_id = client_id.trim().to_string();
    let consumer_key = consumer_key.trim().to_string();
    if client_id.is_empty() || consumer_key.is_empty() {
        return Err("Client ID and Consumer Key are both required.".into());
    }

    SnapTradeClient::new(client_id.clone(), consumer_key.clone())
        .check_credentials()
        .await
        .map_err(friendly)?;

    secrets::set_secret(SNAPTRADE_CONSUMER_KEY, &consumer_key).map_err(|e| e.to_string())?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_setting(&conn, SETTING_CLIENT_ID, &client_id).map_err(|e| e.to_string())?;
    }

    snaptrade_get_status(db)
}

/// Get a connection-portal URL where the user authorizes a brokerage (read-only). Registers
/// the SnapTrade user on first use, and self-heals if the stored user secret was lost.
#[tauri::command]
pub async fn snaptrade_get_login_link(db: State<'_, AppDb>) -> Result<String, String> {
    let (client_id, existing_user_id) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (
            get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?,
            get_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?,
        )
    };
    let client_id = client_id.ok_or("Save your SnapTrade API credentials first.")?;
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;
    let existing_secret = secrets::get_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;

    let client = SnapTradeClient::new(client_id, consumer_key);

    let (user_id, user_secret) = match (existing_user_id, existing_secret) {
        (Some(uid), Some(secret)) => (uid, secret),
        (Some(uid), None) => {
            // We kept the userId but lost its secret — delete (best-effort) and re-register.
            let _ = client.delete_user(&uid).await;
            let secret = client.register_user(&uid).await.map_err(friendly)?;
            secrets::set_secret(SNAPTRADE_USER_SECRET, &secret).map_err(|e| e.to_string())?;
            (uid, secret)
        }
        (None, _) => {
            let uid = generate_user_id();
            let secret = client.register_user(&uid).await.map_err(friendly)?;
            secrets::set_secret(SNAPTRADE_USER_SECRET, &secret).map_err(|e| e.to_string())?;
            {
                let conn = db.0.lock().map_err(|e| e.to_string())?;
                set_setting(&conn, SETTING_USER_ID, &uid).map_err(|e| e.to_string())?;
            }
            (uid, secret)
        }
    };

    client
        .login_link(&user_id, &user_secret)
        .await
        .map_err(friendly)
}

/// Pull accounts + balances + holdings from SnapTrade and reconcile them into the local DB.
/// Writes one balance snapshot per account so net worth updates automatically.
#[tauri::command]
pub async fn snaptrade_sync(db: State<'_, AppDb>) -> Result<SnapTradeSyncSummary, String> {
    let (client_id, user_id) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (
            get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?,
            get_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?,
        )
    };
    let client_id = client_id.ok_or("Save your SnapTrade API credentials first.")?;
    let user_id = user_id.ok_or("Connect a brokerage before syncing.")?;
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;
    let user_secret = secrets::get_secret(SNAPTRADE_USER_SECRET)
        .map_err(|e| e.to_string())?
        .ok_or("Connect a brokerage before syncing.")?;

    let client = SnapTradeClient::new(client_id, consumer_key);

    // Fetch everything over the network first — no DB lock is held across an await.
    let accounts = client
        .list_accounts(&user_id, &user_secret)
        .await
        .map_err(friendly)?;
    let mut synced: Vec<(SnapAccount, Vec<SnapPosition>)> = Vec::with_capacity(accounts.len());
    for account in accounts {
        let positions = client
            .account_positions(&user_id, &user_secret, &account.id)
            .await
            .map_err(friendly)?;
        synced.push((account, positions));
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut accounts_synced = 0usize;
    let mut holdings_synced = 0usize;

    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        for (account, positions) in &synced {
            let currency = account
                .currency
                .clone()
                .unwrap_or_else(|| "USD".to_string());
            let jurisdiction = jurisdiction_for(&currency);
            let account_type =
                map_account_type(account.raw_type.as_deref(), account.name.as_deref());
            let display_name = account
                .name
                .clone()
                .or_else(|| account.number.clone())
                .unwrap_or_else(|| "Brokerage account".to_string());
            let institution = account
                .institution_name
                .clone()
                .unwrap_or_else(|| "SnapTrade".to_string());

            // Upsert the account, keyed by (connector_kind, connector_ref).
            let existing_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM accounts WHERE connector_kind = 'snaptrade' AND connector_ref = ?1",
                    params![account.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;

            let account_id = if let Some(id) = existing_id {
                tx.execute(
                    "UPDATE accounts SET name = ?1, institution = ?2, account_type = ?3, \
                     currency = ?4, jurisdiction = ?5, is_active = 1, updated_at = ?6 WHERE id = ?7",
                    params![display_name, institution, account_type, currency, jurisdiction, now, id],
                )
                .map_err(|e| e.to_string())?;
                id
            } else {
                tx.execute(
                    "INSERT INTO accounts \
                     (name, institution, account_type, currency, jurisdiction, connector_kind, connector_ref) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'snaptrade', ?6)",
                    params![display_name, institution, account_type, currency, jurisdiction, account.id],
                )
                .map_err(|e| e.to_string())?;
                tx.last_insert_rowid()
            };
            accounts_synced += 1;

            // Balance snapshot → picked up by the net-worth pipeline.
            if let Some(total) = account.balance_total {
                tx.execute(
                    "INSERT OR REPLACE INTO balance_snapshots \
                     (account_id, snapshot_date, balance, currency, source) \
                     VALUES (?1, ?2, ?3, ?4, 'snaptrade')",
                    params![account_id, today, total, currency],
                )
                .map_err(|e| e.to_string())?;
            }

            // Replace the holdings set for this account so closed positions disappear.
            tx.execute(
                "DELETE FROM holdings WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
            for pos in positions {
                let holding_currency = pos.currency.clone().unwrap_or_else(|| currency.clone());
                let last_price_at = pos.price.map(|_| now.clone());
                tx.execute(
                    "INSERT OR REPLACE INTO holdings \
                     (account_id, symbol, quantity, average_cost, currency, last_price, last_price_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        account_id,
                        pos.symbol,
                        pos.units,
                        pos.average_purchase_price,
                        holding_currency,
                        pos.price,
                        last_price_at,
                        now
                    ],
                )
                .map_err(|e| e.to_string())?;
                holdings_synced += 1;
            }
        }

        set_setting(&tx, SETTING_LAST_SYNCED, &now).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(SnapTradeSyncSummary {
        accounts_synced,
        holdings_synced,
        synced_at: now,
    })
}

/// Disconnect the brokerage: delete the SnapTrade user remotely, clear the local user secret
/// + identifiers, and deactivate the connected accounts. API credentials are kept so the user
/// can reconnect without re-entering them.
#[tauri::command]
pub async fn snaptrade_disconnect(db: State<'_, AppDb>) -> Result<SnapTradeStatus, String> {
    let (client_id, user_id) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (
            get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?,
            get_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?,
        )
    };
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY).map_err(|e| e.to_string())?;

    // Best-effort remote delete; proceed with local cleanup regardless.
    if let (Some(cid), Some(uid), Some(ck)) = (client_id, user_id, consumer_key) {
        let _ = SnapTradeClient::new(cid, ck).delete_user(&uid).await;
    }

    secrets::delete_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        delete_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?;
        delete_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
             WHERE connector_kind = 'snaptrade'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    snaptrade_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_registered_account_types_from_keywords() {
        assert_eq!(map_account_type(Some("Roth IRA"), None), "roth_ira");
        assert_eq!(map_account_type(Some("Traditional IRA"), None), "ira");
        assert_eq!(map_account_type(Some("401(k)"), None), "401k");
        assert_eq!(map_account_type(Some("TFSA"), None), "tfsa");
        assert_eq!(map_account_type(Some("RRSP"), None), "rrsp");
        assert_eq!(map_account_type(Some("FHSA"), None), "fhsa");
        assert_eq!(
            map_account_type(None, Some("My Margin Account")),
            "brokerage"
        );
        assert_eq!(map_account_type(Some("Individual"), None), "brokerage");
    }

    #[test]
    fn roth_takes_priority_over_plain_ira() {
        // "Roth IRA" contains "IRA" too; the more specific match must win.
        assert_eq!(
            map_account_type(Some("ROTH IRA"), Some("Retirement")),
            "roth_ira"
        );
    }

    #[test]
    fn jurisdiction_follows_currency() {
        assert_eq!(jurisdiction_for("CAD"), "CA");
        assert_eq!(jurisdiction_for("cad"), "CA");
        assert_eq!(jurisdiction_for("USD"), "US");
        assert_eq!(jurisdiction_for("EUR"), "US");
    }

    #[test]
    fn generated_user_id_is_prefixed_and_unique() {
        let a = generate_user_id();
        let b = generate_user_id();
        assert!(a.starts_with("truenorth-"));
        assert_ne!(a, b);
    }

    #[test]
    fn settings_roundtrip_and_delete() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        assert_eq!(get_setting(&conn, SETTING_CLIENT_ID).unwrap(), None);
        set_setting(&conn, SETTING_CLIENT_ID, "client-123").unwrap();
        assert_eq!(
            get_setting(&conn, SETTING_CLIENT_ID).unwrap().as_deref(),
            Some("client-123")
        );
        // Upsert overwrites.
        set_setting(&conn, SETTING_CLIENT_ID, "client-456").unwrap();
        assert_eq!(
            get_setting(&conn, SETTING_CLIENT_ID).unwrap().as_deref(),
            Some("client-456")
        );
        delete_setting(&conn, SETTING_CLIENT_ID).unwrap();
        assert_eq!(get_setting(&conn, SETTING_CLIENT_ID).unwrap(), None);
    }
}
