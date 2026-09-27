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

use super::account_selection::{
    self, AccountReview, Provider, RemoteAccount, SaveAccountChoices, SyncPlan,
};
use crate::connector::snaptrade::{SnapAccount, SnapPosition, SnapTradeClient, SnapTradeError};
use crate::db::secrets::{self, SNAPTRADE_CONSUMER_KEY, SNAPTRADE_USER_SECRET};
use crate::db::{reconcile_aggregated_questrade_accounts, AppDb};

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
    /// The clientId is a SnapTrade *personal* key (`PERS-…`): its user is auto-provisioned at
    /// signup and `registerUser` is unavailable, so the user links userId + userSecret manually.
    pub is_personal: bool,
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
    pub accounts_needing_review: usize,
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

pub(super) fn remote_accounts(accounts: &[SnapAccount]) -> Vec<RemoteAccount> {
    accounts
        .iter()
        .map(|a| RemoteAccount {
            remote_id: a.id.clone(),
            name: a.name.clone().unwrap_or_else(|| "Brokerage account".into()),
            institution: a.institution_name.clone().unwrap_or_else(|| "SnapTrade".into()),
            account_type: map_account_type(a.raw_type.as_deref(), a.name.as_deref()),
            currency: a.currency.as_deref().map(|c| c.trim().to_ascii_uppercase()),
            masked_number: account_selection::mask_account_number(a.number.as_deref()),
        })
        .collect()
}

struct SnapConnection {
    client: SnapTradeClient,
    client_id: String,
    user_id: String,
    user_secret: String,
}

fn load_connection(db: &AppDb) -> Result<SnapConnection, String> {
    let (client_id, user_id) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (
            get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?,
            get_setting(&conn, SETTING_USER_ID).map_err(|e| e.to_string())?,
        )
    };
    let client_id = client_id.ok_or("Save your SnapTrade API credentials first.")?;
    let user_id = user_id.ok_or("Connect a brokerage before choosing accounts or syncing.")?;
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;
    let user_secret = secrets::get_secret(SNAPTRADE_USER_SECRET)
        .map_err(|e| e.to_string())?
        .ok_or("Connect a brokerage before choosing accounts or syncing.")?;
    Ok(SnapConnection {
        client: SnapTradeClient::new(client_id.clone(), consumer_key),
        client_id,
        user_id,
        user_secret,
    })
}

fn check_connection(conn: &Connection, source: &SnapConnection) -> Result<(), String> {
    if get_setting(conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?.as_deref()
        != Some(&source.client_id)
        || get_setting(conn, SETTING_USER_ID).map_err(|e| e.to_string())?.as_deref()
            != Some(&source.user_id)
    {
        return Err("The SnapTrade connection changed. Reopen Choose accounts or sync again.".into());
    }
    Ok(())
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

/// SnapTrade "personal" API keys have a `PERS-` clientId prefix. Their single user is
/// auto-provisioned at signup and `registerUser` returns 400, so the connect flow branches on
/// this: personal keys link an existing userId + userSecret instead of registering one.
fn is_personal_key(client_id: &str) -> bool {
    client_id.trim().to_ascii_uppercase().starts_with("PERS-")
}

/// Shown when a personal key has no linked user yet and the user tries to open the login portal.
const PERSONAL_LINK_HINT: &str =
    "This is a personal SnapTrade key. Open the SnapTrade dashboard, copy your User ID and User \
     Secret, and paste them in the “SnapTrade user” step before connecting a brokerage.";

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
        let account_count = account_selection::active_count(&conn, Provider::SnapTrade)?;
        (client_id, user_id, last_synced_at, account_count)
    };

    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY).map_err(|e| e.to_string())?;
    let user_secret = secrets::get_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;

    Ok(SnapTradeStatus {
        has_credentials: client_id.is_some() && consumer_key.is_some(),
        is_connected: user_id.is_some() && user_secret.is_some(),
        is_personal: client_id.as_deref().map(is_personal_key).unwrap_or(false),
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
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let previous = get_setting(&tx, SETTING_CLIENT_ID).map_err(|e| e.to_string())?;
        if previous.as_deref().is_some_and(|id| id != client_id) {
            account_selection::pause_provider(&tx, Provider::SnapTrade)?;
            delete_setting(&tx, SETTING_USER_ID).map_err(|e| e.to_string())?;
            delete_setting(&tx, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        }
        set_setting(&tx, SETTING_CLIENT_ID, &client_id).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    snaptrade_get_status(db)
}

/// List the SnapTrade user IDs registered under the saved API key. For a personal key this is
/// the single user SnapTrade auto-provisioned at signup; the UI uses it to prefill the User ID.
#[tauri::command]
pub async fn snaptrade_list_users(db: State<'_, AppDb>) -> Result<Vec<String>, String> {
    let client_id = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?
    }
    .ok_or("Save your SnapTrade API credentials first.")?;
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;
    SnapTradeClient::new(client_id, consumer_key)
        .list_users()
        .await
        .map_err(friendly)
}

/// Link a SnapTrade user by `userId` + `userSecret`. This is the connect path for personal keys:
/// their user is created automatically at signup (so `registerUser` is unavailable), and the
/// user copies both values from the SnapTrade dashboard. We validate them by listing accounts
/// (401/403 → wrong values) before storing the secret in the keychain and the userId in settings.
#[tauri::command]
pub async fn snaptrade_link_user(
    db: State<'_, AppDb>,
    user_id: String,
    user_secret: String,
) -> Result<SnapTradeStatus, String> {
    let user_id = user_id.trim().to_string();
    let user_secret = user_secret.trim().to_string();
    if user_id.is_empty() || user_secret.is_empty() {
        return Err("User ID and User Secret are both required.".into());
    }

    let client_id = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        get_setting(&conn, SETTING_CLIENT_ID).map_err(|e| e.to_string())?
    }
    .ok_or("Save your SnapTrade API credentials first.")?;
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;

    // Validate the pair before persisting. A user with no connections yet returns an empty list
    // (still HTTP 200), which is fine — it just means nothing is linked at SnapTrade yet.
    SnapTradeClient::new(client_id, consumer_key)
        .list_accounts(&user_id, &user_secret)
        .await
        .map_err(|e| {
            if e.is_auth() {
                "SnapTrade rejected those credentials. Double-check the User ID and User Secret \
                 from your dashboard."
                    .to_string()
            } else {
                friendly(e)
            }
        })?;

    secrets::set_secret(SNAPTRADE_USER_SECRET, &user_secret).map_err(|e| e.to_string())?;
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let previous = get_setting(&tx, SETTING_USER_ID).map_err(|e| e.to_string())?;
        if previous.as_deref().is_some_and(|id| id != user_id) {
            account_selection::pause_provider(&tx, Provider::SnapTrade)?;
            delete_setting(&tx, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        }
        set_setting(&tx, SETTING_USER_ID, &user_id).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    snaptrade_get_status(db)
}

/// Get a connection-portal URL where the user authorizes a brokerage (read-only). For commercial
/// keys this registers the SnapTrade user on first use (self-healing a lost secret). For personal
/// keys the user must link their userId + userSecret first (see `snaptrade_link_user`).
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
    let personal = is_personal_key(&client_id);
    let consumer_key = secrets::get_secret(SNAPTRADE_CONSUMER_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("Save your SnapTrade API credentials first.")?;
    let existing_secret = secrets::get_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;

    let client = SnapTradeClient::new(client_id, consumer_key);

    let (user_id, user_secret) = match (existing_user_id, existing_secret) {
        (Some(uid), Some(secret)) => (uid, secret),
        // Personal keys can't registerUser: the user must paste userId + userSecret first.
        _ if personal => return Err(PERSONAL_LINK_HINT.into()),
        (Some(uid), None) => {
            // Commercial key: we kept the userId but lost its secret — re-register.
            let _ = client.delete_user(&uid).await;
            let secret = client.register_user(&uid).await.map_err(friendly)?;
            secrets::set_secret(SNAPTRADE_USER_SECRET, &secret).map_err(|e| e.to_string())?;
            (uid, secret)
        }
        (None, _) => {
            // Commercial key: first connect — register a fresh user.
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

/// Discover provider metadata and saved choices without importing financial data.
#[tauri::command]
pub async fn snaptrade_discover_accounts(db: State<'_, AppDb>) -> Result<AccountReview, String> {
    let source = load_connection(&db)?;
    let accounts = source.client
        .list_accounts(&source.user_id, &source.user_secret)
        .await
        .map_err(friendly)?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    check_connection(&conn, &source)?;
    account_selection::discover(&conn, Provider::SnapTrade, &remote_accounts(&accounts))
}

#[tauri::command]
pub async fn snaptrade_save_account_choices(
    db: State<'_, AppDb>,
    payload: SaveAccountChoices,
) -> Result<AccountReview, String> {
    let source = load_connection(&db)?;
    let accounts = source.client
        .list_accounts(&source.user_id, &source.user_secret)
        .await
        .map_err(friendly)?;
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    check_connection(&conn, &source)?;
    account_selection::save_choices(&mut conn, Provider::SnapTrade, &remote_accounts(&accounts), &payload)
}

pub(super) fn apply_sync(
    conn: &mut Connection,
    remotes: &[RemoteAccount],
    plan: &SyncPlan,
    synced: &[(SnapAccount, Vec<SnapPosition>)],
    today: &str,
    now: &str,
) -> Result<SnapTradeSyncSummary, String> {
    plan.require_selected()?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    account_selection::validate_sync(&tx, Provider::SnapTrade, remotes, plan)?;
    let mut holdings_synced = 0;
    for target in &plan.targets {
        let (account, positions) = synced.iter().find(|(a, _)| a.id == target.remote_id)
            .ok_or("SnapTrade did not return a selected account. Nothing was imported.")?;
        if let Some(total) = account.balance_total {
            tx.execute(
                "INSERT OR REPLACE INTO balance_snapshots \
                 (account_id, snapshot_date, balance, currency, source) \
                 VALUES (?1, ?2, ?3, ?4, 'snaptrade')",
                params![target.account_id, today, total, target.currency],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM holdings WHERE account_id = ?1", [target.account_id])
            .map_err(|e| e.to_string())?;
        for pos in positions {
            let holding_currency = pos.currency.as_deref().unwrap_or(&target.currency);
            let last_price_at = pos.price.map(|_| now);
            tx.execute(
                "INSERT OR REPLACE INTO holdings \
                 (account_id, symbol, quantity, average_cost, currency, last_price, last_price_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![target.account_id, pos.symbol, pos.units, pos.average_purchase_price,
                    holding_currency, pos.price, last_price_at, now],
            )
            .map_err(|e| e.to_string())?;
            holdings_synced += 1;
        }
    }
    reconcile_aggregated_questrade_accounts(&tx).map_err(|e| e.to_string())?;
    set_setting(&tx, SETTING_LAST_SYNCED, now).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(SnapTradeSyncSummary {
        accounts_synced: plan.targets.len(),
        holdings_synced,
        synced_at: now.to_string(),
        accounts_needing_review: plan.accounts_needing_review,
    })
}

/// Fetch positions only for selected accounts, then recheck ownership before writing.
async fn fetch_selected_positions(
    source: &SnapConnection,
    accounts: &[SnapAccount],
    plan: &SyncPlan,
) -> Result<Vec<(SnapAccount, Vec<SnapPosition>)>, String> {
    plan.require_selected()?;
    let mut synced = Vec::with_capacity(plan.targets.len());
    for target in &plan.targets {
        let account = accounts.iter().find(|a| a.id == target.remote_id)
            .ok_or("A selected SnapTrade account is no longer available.")?;
        let positions = source.client
            .account_positions(&source.user_id, &source.user_secret, &target.remote_id)
            .await
            .map_err(friendly)?;
        synced.push((account.clone(), positions));
    }
    Ok(synced)
}

#[tauri::command]
pub async fn snaptrade_sync(db: State<'_, AppDb>) -> Result<SnapTradeSyncSummary, String> {
    let source = load_connection(&db)?;
    let accounts = source.client
        .list_accounts(&source.user_id, &source.user_secret)
        .await
        .map_err(friendly)?;
    let remotes = remote_accounts(&accounts);
    let plan = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        check_connection(&conn, &source)?;
        account_selection::plan_sync(&conn, Provider::SnapTrade, &remotes)?
    };
    let synced = fetch_selected_positions(&source, &accounts, &plan).await?;
    let now = chrono::Utc::now();
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    check_connection(&conn, &source)?;
    apply_sync(
        &mut conn, &remotes, &plan, &synced,
        &now.format("%Y-%m-%d").to_string(),
        &now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
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

    // Best-effort remote delete — commercial keys only. A personal key's user is provisioned at
    // signup and owns the user's own brokerage connections (managed in the SnapTrade dashboard),
    // so deleting it would wipe their real connections. For personal keys we clear local state only.
    if let (Some(cid), Some(uid), Some(ck)) = (client_id, user_id, consumer_key) {
        if !is_personal_key(&cid) {
            let _ = SnapTradeClient::new(cid, ck).delete_user(&uid).await;
        }
    }

    secrets::delete_secret(SNAPTRADE_USER_SECRET).map_err(|e| e.to_string())?;
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        delete_setting(&tx, SETTING_USER_ID).map_err(|e| e.to_string())?;
        delete_setting(&tx, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        account_selection::pause_provider(&tx, Provider::SnapTrade)?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    snaptrade_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::accounts::aggregated_account_jurisdiction;

    #[tokio::test]
    async fn positions_are_requested_only_for_selected_accounts() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") { break; }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let source = SnapConnection {
            client: SnapTradeClient::new("synthetic-client", "synthetic-key")
                .with_base_url(format!("http://{address}")),
            client_id: "synthetic-client".into(),
            user_id: "synthetic-user".into(),
            user_secret: "synthetic-secret".into(),
        };
        let accounts: Vec<SnapAccount> = ["selected", "ignored", "unreviewed"].into_iter().map(|id| SnapAccount {
            id: id.into(), name: Some("Individual".into()), number: None,
            institution_name: Some("Robinhood".into()), raw_type: Some("Individual".into()),
            balance_total: Some(100.0), currency: Some("USD".into()),
        }).collect();
        let remotes = remote_accounts(&accounts);
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let revision = account_selection::discover(&conn, Provider::SnapTrade, &remotes).unwrap().revision;
        account_selection::save_choices(&mut conn, Provider::SnapTrade, &remotes, &SaveAccountChoices {
            revision, choices: vec![
                account_selection::AccountChoice::Create { remote_id: "selected".into() },
                account_selection::AccountChoice::Ignore { remote_id: "ignored".into() },
            ],
        }).unwrap();
        let plan = account_selection::plan_sync(&conn, Provider::SnapTrade, &remotes).unwrap();
        let synced = fetch_selected_positions(&source, &accounts, &plan).await.unwrap();
        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].0.id, "selected");
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), server).await.unwrap().unwrap();
        assert!(request.starts_with("GET /api/v1/accounts/selected/positions?"));
        assert!(!request.contains("ignored"));
        assert!(!request.contains("unreviewed"));
        assert_eq!(plan.accounts_needing_review, 1);
    }

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
    fn jurisdiction_uses_currency_except_for_questrade() {
        assert_eq!(aggregated_account_jurisdiction("CAD", None), "CA");
        assert_eq!(aggregated_account_jurisdiction("cad", None), "CA");
        assert_eq!(aggregated_account_jurisdiction("USD", None), "US");
        assert_eq!(aggregated_account_jurisdiction("EUR", None), "US");
        assert_eq!(
            aggregated_account_jurisdiction("USD", Some("Questrade")),
            "CA"
        );
    }

    #[test]
    fn generated_user_id_is_prefixed_and_unique() {
        let a = generate_user_id();
        let b = generate_user_id();
        assert!(a.starts_with("truenorth-"));
        assert_ne!(a, b);
    }

    #[test]
    fn detects_personal_keys_by_prefix() {
        assert!(is_personal_key("PERS-5IH4YWHEHYX9G70CZELD"));
        assert!(is_personal_key("  pers-lowercase-trimmed  "));
        assert!(!is_personal_key("CLIENTID-COMMERCIAL"));
        assert!(!is_personal_key("truenorth"));
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
