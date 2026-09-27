//! SimpleFIN Tauri commands: claim a setup token, sync balances + holdings, disconnect.
//!
//! SimpleFIN needs no signing or user registration — the claimed **access URL** (stored in the
//! keychain) is all that's required. As with SnapTrade, the SQLite mutex is never held across an
//! `.await`: network calls happen first, then results are written under a short-lived lock, and
//! one balance snapshot per account flows straight into the existing net-worth pipeline.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use super::account_selection::{
    self, AccountReview, Provider, RemoteAccount, SaveAccountChoices, SyncPlan,
};
use super::simplefin_health::{self as health, ConnectionHealth};
use crate::connector::simplefin::{
    claim_access_url, SimpleFinAccount, SimpleFinAccountSet, SimpleFinClient, SimpleFinError,
    SimpleFinHolding, SimpleFinTransaction,
};
use crate::db::secrets::{self, SIMPLEFIN_ACCESS_URL};
use crate::db::{reconcile_aggregated_questrade_accounts, AppDb};

const SETTING_LAST_SYNCED: &str = "simplefin_last_synced_at";

#[derive(Default)]
pub struct SimpleFinSyncLock(pub tokio::sync::Mutex<()>);

// ---------------------------------------------------------------------------
// Serialisable types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SimpleFinStatus {
    /// An access URL is stored (the user has claimed a setup token).
    pub is_connected: bool,
    pub last_synced_at: Option<String>,
    /// Number of active accounts connected via SimpleFIN.
    pub account_count: i64,
    pub last_attempt_at: Option<String>,
    pub app_auth_required: bool,
    pub messages: Vec<String>,
    pub connections: Vec<ConnectionHealth>,
}

#[derive(Debug, Serialize)]
pub struct SimpleFinSyncSummary {
    pub accounts_synced: usize,
    pub holdings_synced: usize,
    pub transactions_synced: usize,
    pub synced_at: Option<String>,
    pub accounts_needing_review: usize,
    /// Non-fatal messages SimpleFIN returned (e.g. one institution needs to be re-authenticated).
    pub warnings: Vec<String>,
    pub skipped: bool,
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

/// Best-effort map from a SimpleFIN account name (banks rarely report a type) to one of
/// TrueNorth's account-type ids. Accounts that report holdings are treated as brokerage.
fn map_account_type(name: &str, has_holdings: bool) -> String {
    let hay = name.to_uppercase();
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
    } else if has("CREDIT") || has("VISA") || has("MASTERCARD") || has("CARD") {
        "credit"
    } else if has("CHEQUING") || has("CHECKING") {
        "chequing"
    } else if has("SAVING") {
        "savings"
    } else if has("CRYPTO") {
        "crypto"
    } else if has_holdings || has("BROKERAGE") || has("INVEST") {
        "brokerage"
    } else {
        // SimpleFIN is bank-focused; default a plain deposit account to chequing.
        "chequing"
    };
    kind.to_string()
}

pub(super) fn remote_accounts(accounts: &[SimpleFinAccount]) -> Vec<RemoteAccount> {
    accounts
        .iter()
        .map(|a| RemoteAccount {
            remote_id: a.reference(),
            connection_id: a.connection_id.clone(),
            name: a.name.clone(),
            institution: a.institution.clone().unwrap_or_else(|| "SimpleFIN".into()),
            account_type: map_account_type(&a.name, !a.holdings.is_empty()),
            currency: Some(a.currency.trim().to_ascii_uppercase()).filter(|c| !c.is_empty()),
            masked_number: account_selection::mask_account_number(a.number.as_deref()),
        })
        .collect()
}

fn review_accounts(
    conn: &Connection,
    accounts: &[SimpleFinAccount],
) -> Result<Vec<RemoteAccount>, String> {
    let mut remotes = remote_accounts(accounts);
    for (account, remote) in accounts.iter().zip(&mut remotes) {
        let canonical: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_sync_selections WHERE provider = 'simplefin' AND remote_id = ?1)",
            [&remote.remote_id], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if !canonical && account.connection_id.is_some() {
            let legacy: Option<Option<String>> = conn.query_row(
                "SELECT connection_id FROM account_sync_selections WHERE provider = 'simplefin' AND remote_id = ?1",
                [&account.id], |r| r.get(0),
            ).optional().map_err(|e| e.to_string())?;
            if legacy.is_some_and(|connection| {
                connection.is_none() || connection == account.connection_id
            }) {
                if accounts.iter().filter(|a| a.id == account.id).count() != 1 {
                    return Err("A legacy SimpleFIN account ID is shared by multiple bank connections. No accounts were imported; its existing history needs explicit reconciliation.".into());
                }
                // Preserve an exact, unambiguous legacy identity (including exclusions) without
                // changing financial accounts during discovery. New identities remain scoped.
                remote.remote_id = account.id.clone();
            }
        }
    }
    Ok(remotes)
}

fn matches_remote(account: &SimpleFinAccount, remote: &RemoteAccount) -> bool {
    account.connection_id == remote.connection_id
        && (account.reference() == remote.remote_id || account.id == remote.remote_id)
}

fn access_url() -> Result<String, String> {
    secrets::get_secret(SIMPLEFIN_ACCESS_URL)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Connect SimpleFIN before choosing accounts or syncing.".into())
}

fn check_connection(expected_url: &str) -> Result<(), String> {
    if access_url()? != expected_url {
        return Err(
            "The SimpleFIN connection changed. Reopen Choose accounts or sync again.".into(),
        );
    }
    Ok(())
}

/// Turn a SimpleFIN error into a user-facing message.
fn friendly(e: SimpleFinError) -> String {
    if e.is_auth() {
        "SimpleFIN rejected the access URL. Reconnect with a new setup token from your SimpleFIN \
         bridge (and disable the old one if you think it was exposed)."
            .into()
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// DB reconcile helpers (synchronous — never run while awaiting)
// ---------------------------------------------------------------------------

/// Missing or invalid provider dates must never become fresh "today" snapshots.
fn snapshot_date_for(balance_date: Option<i64>, today: &str) -> Option<String> {
    balance_date
        .filter(|at| *at > 0)
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        .map(|at| at.format("%Y-%m-%d").to_string())
        .filter(|date| date.as_str() <= today)
}

/// Update only a selected account's observed balance, preserving its identity and metadata.
fn write_balance_snapshot(
    conn: &Connection,
    account_id: i64,
    currency: &str,
    account: &SimpleFinAccount,
    today: &str,
    now: &str,
) -> rusqlite::Result<()> {
    let checked_at = chrono::DateTime::parse_from_rfc3339(now)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .with_timezone(&chrono::Utc);
    let source_time = health::source_time(account, checked_at);
    let previous_time = health::load(conn)?.balance_at(account_id);
    if let (Some(total), Some(snapshot_date), Some(source_time)) = (
        account.balance,
        snapshot_date_for(account.balance_date, today),
        source_time,
    ) {
        if previous_time.is_some_and(|previous| source_time < previous) {
            return Ok(());
        }
        conn.execute(
            "INSERT OR REPLACE INTO balance_snapshots \
             (account_id, snapshot_date, balance, currency, source) \
             VALUES (?1, ?2, ?3, ?4, 'simplefin')",
            params![account_id, snapshot_date, total, currency],
        )?;
    }

    Ok(())
}

/// SimpleFIN reports `market_value`/`cost_basis` as position totals; derive the per-share
/// `(last_price, average_cost)` the holdings table stores. Guards against a zero-share divide.
fn holding_unit_prices(h: &SimpleFinHolding) -> (Option<f64>, Option<f64>) {
    if h.shares != 0.0 {
        (
            h.market_value.map(|mv| mv / h.shares),
            h.cost_basis.map(|cb| cb / h.shares),
        )
    } else {
        (None, None)
    }
}

/// Replace an account's holdings so closed positions disappear. Returns the number inserted.
fn replace_holdings(
    conn: &Connection,
    account_id: i64,
    account: &SimpleFinAccount,
    account_currency: &str,
    now: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM holdings WHERE account_id = ?1",
        params![account_id],
    )?;
    let mut count = 0usize;
    for h in &account.holdings {
        let (last_price, average_cost) = holding_unit_prices(h);
        let holding_currency = h.currency.as_deref().unwrap_or(account_currency);
        let last_price_at = last_price.map(|_| now.to_string());
        conn.execute(
            "INSERT OR REPLACE INTO holdings \
             (account_id, symbol, quantity, average_cost, currency, last_price, last_price_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                h.symbol,
                h.shares,
                average_cost,
                holding_currency,
                last_price,
                last_price_at,
                now
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Convert a UNIX epoch (seconds) to an ISO `YYYY-MM-DD` date, falling back to `fallback` when the
/// timestamp is missing or out of range.
fn epoch_to_date(epoch: Option<i64>, fallback: &str) -> String {
    epoch
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// Upsert a set of transactions, keyed by `(account_id, connector_ref)` so re-syncs update in
/// place instead of duplicating. The user's classification (`category`, `flow_override`) is left
/// untouched on update. Returns the number processed.
fn reconcile_transactions(
    conn: &Connection,
    account_id: i64,
    transactions: &[SimpleFinTransaction],
    currency: &str,
    today: &str,
) -> rusqlite::Result<usize> {
    let mut count = 0usize;
    for t in transactions {
        let date = epoch_to_date(t.posted, today);
        conn.execute(
            "INSERT INTO transactions \
             (account_id, txn_date, description, amount, currency, memo, connector_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(account_id, connector_ref) DO UPDATE SET \
                 txn_date = excluded.txn_date, description = excluded.description, \
                 amount = excluded.amount, currency = excluded.currency, memo = excluded.memo",
            params![
                account_id,
                date,
                t.description,
                t.amount,
                currency,
                t.memo,
                t.id
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Report whether SimpleFIN is connected, plus last-synced time and connected-account count.
#[tauri::command]
pub fn simplefin_get_status(db: State<AppDb>) -> Result<SimpleFinStatus, String> {
    let access_url = secrets::get_secret(SIMPLEFIN_ACCESS_URL).map_err(|e| e.to_string())?;
    let is_connected = access_url.is_some();
    let (last_synced_at, account_count) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let last_synced_at = get_setting(&conn, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        let account_count = account_selection::active_count(&conn, Provider::SimpleFin)?;
        (last_synced_at, account_count)
    };

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let state = health::load(&conn).map_err(|e| e.to_string())?;
    let connections = state
        .connections(&conn, is_connected, chrono::Utc::now())
        .map_err(|e| e.to_string())?;
    let messages = state.messages();

    Ok(SimpleFinStatus {
        is_connected,
        last_synced_at,
        account_count,
        last_attempt_at: state.last_attempt_at,
        app_auth_required: state.app_auth_required,
        messages,
        connections,
    })
}

/// Claim a SimpleFIN setup token, validate the resulting access URL, and store it in the keychain.
#[tauri::command]
pub async fn simplefin_connect(
    db: State<'_, AppDb>,
    sync: State<'_, SimpleFinSyncLock>,
    setup_token: String,
) -> Result<SimpleFinStatus, String> {
    let _guard = sync.0.lock().await;
    let setup_token = setup_token.trim().to_string();
    if setup_token.is_empty() {
        return Err("Paste the setup token from your SimpleFIN bridge first.".into());
    }

    // Persist a claimed token before fetching. An outage must not lose a one-time credential.
    let access_url = claim_access_url(&setup_token).await.map_err(friendly)?;
    let previous = secrets::get_secret(SIMPLEFIN_ACCESS_URL).map_err(|e| e.to_string())?;
    secrets::set_secret(SIMPLEFIN_ACCESS_URL, &access_url).map_err(|e| e.to_string())?;
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if previous.as_deref().is_some_and(|url| url != access_url) {
            account_selection::pause_provider(&tx, Provider::SimpleFin)?;
            delete_setting(&tx, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        }
        let mut state = health::load(&tx).map_err(|e| e.to_string())?;
        state.token_saved();
        health::save(&tx, &state).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    simplefin_get_status(db)
}

fn reserve_request(db: &AppDb) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut state = health::load(&conn).map_err(|e| e.to_string())?;
    state.reserve_request(chrono::Utc::now())?;
    health::save(&conn, &state).map_err(|e| e.to_string())
}

fn record_failure(db: &AppDb, message: String, auth: bool) -> String {
    let save = (|| -> Result<(), String> {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut state = health::load(&conn).map_err(|e| e.to_string())?;
        state.app_auth_required |= auth;
        state.last_error = Some(message.clone());
        health::save(&conn, &state).map_err(|e| e.to_string())
    })();
    match save {
        Ok(()) => message,
        Err(error) => format!("{message} Could not save connection status: {error}"),
    }
}

fn record_provider_failure(db: &AppDb, error: SimpleFinError) -> String {
    let auth = error.is_auth();
    record_failure(db, friendly(error), auth)
}

async fn fetch_catalog(db: &AppDb, url: &str) -> Result<SimpleFinAccountSet, String> {
    let catalog = SimpleFinClient::new(url)
        .map_err(|e| record_provider_failure(db, e))?
        .list_accounts()
        .await
        .map_err(|e| record_provider_failure(db, e))?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut state = health::load(&conn).map_err(|e| e.to_string())?;
    state.review_response(&catalog);
    health::save(&conn, &state).map_err(|e| e.to_string())?;
    Ok(catalog)
}

#[tauri::command]
pub async fn simplefin_discover_accounts(
    db: State<'_, AppDb>,
    sync: State<'_, SimpleFinSyncLock>,
) -> Result<AccountReview, String> {
    let _guard = sync.0.lock().await;
    let url = access_url()?;
    reserve_request(&db)?;
    let catalog = fetch_catalog(&db, &url).await?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    check_connection(&url)?;
    let remotes = review_accounts(&conn, &catalog.accounts)?;
    let mut review = account_selection::discover(&conn, Provider::SimpleFin, &remotes)?;
    review.warnings = catalog.errors;
    Ok(review)
}

#[tauri::command]
pub async fn simplefin_save_account_choices(
    db: State<'_, AppDb>,
    sync: State<'_, SimpleFinSyncLock>,
    payload: SaveAccountChoices,
) -> Result<AccountReview, String> {
    let _guard = sync.0.lock().await;
    let url = access_url()?;
    reserve_request(&db)?;
    let catalog = fetch_catalog(&db, &url).await?;
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    check_connection(&url)?;
    let remotes = review_accounts(&conn, &catalog.accounts)?;
    let mut review =
        account_selection::save_choices(&mut conn, Provider::SimpleFin, &remotes, &payload)?;
    review.warnings = catalog.errors;
    Ok(review)
}

pub(super) fn apply_sync(
    conn: &mut Connection,
    remotes: &[RemoteAccount],
    plan: &SyncPlan,
    account_set: &SimpleFinAccountSet,
    today: &str,
    now: &str,
) -> Result<SimpleFinSyncSummary, String> {
    plan.require_selected()?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    account_selection::validate_sync(&tx, Provider::SimpleFin, remotes, plan)?;
    let mut returned_ids = HashSet::new();
    for account in &account_set.accounts {
        if account.id.trim().is_empty() || !returned_ids.insert(account.reference()) {
            return Err(
                "SimpleFIN returned missing or duplicate account IDs. Nothing was imported.".into(),
            );
        }
    }
    let checked_at = chrono::DateTime::parse_from_rfc3339(now)
        .map_err(|e| e.to_string())?
        .with_timezone(&chrono::Utc);
    let mut state = health::load(&tx).map_err(|e| e.to_string())?;
    state.response(account_set, checked_at);
    let mut holdings_synced = 0;
    let mut transactions_synced = 0;
    for target in &plan.targets {
        let expected = remotes
            .iter()
            .find(|a| a.remote_id == target.remote_id)
            .ok_or("The selected account is missing from the reviewed catalog.")?;
        let account = account_set
            .accounts
            .iter()
            .find(|a| matches_remote(a, expected))
            .ok_or_else(|| {
                format!(
                    "SimpleFIN did not return a selected account. Nothing was imported. {}",
                    account_set.errors.join(" ")
                )
            })?;
        if !account
            .currency
            .trim()
            .eq_ignore_ascii_case(expected.currency.as_deref().unwrap_or(""))
        {
            return Err("SimpleFIN changed an account's reported currency during sync. Nothing was imported; review accounts again.".into());
        }
        tx.execute(
            "UPDATE account_sync_selections SET connection_id = ?1 \
             WHERE provider = 'simplefin' AND remote_id = ?2 AND connection_id IS NULL",
            params![expected.connection_id, target.remote_id],
        )
        .map_err(|e| e.to_string())?;
        let failed = health::account_data_failed(account_set, account);
        let mut safe_account = account.clone();
        if failed {
            safe_account.balance = None;
            safe_account.holdings_reported = false;
        }
        write_balance_snapshot(
            &tx,
            target.account_id,
            &target.currency,
            &safe_account,
            today,
            now,
        )
        .map_err(|e| e.to_string())?;
        if account.holdings_reported && !failed {
            holdings_synced +=
                replace_holdings(&tx, target.account_id, account, &target.currency, now)
                    .map_err(|e| e.to_string())?;
        }
        transactions_synced += reconcile_transactions(
            &tx,
            target.account_id,
            &account.transactions,
            &target.currency,
            today,
        )
        .map_err(|e| e.to_string())?;
        state.record_account(target.account_id, &safe_account, checked_at);
    }
    reconcile_aggregated_questrade_accounts(&tx).map_err(|e| e.to_string())?;
    set_setting(&tx, SETTING_LAST_SYNCED, now).map_err(|e| e.to_string())?;
    state.finish_response();
    health::save(&tx, &state).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(SimpleFinSyncSummary {
        accounts_synced: plan.targets.len(),
        holdings_synced,
        transactions_synced,
        synced_at: Some(now.to_string()),
        accounts_needing_review: plan.accounts_needing_review,
        warnings: account_set.errors.clone(),
        skipped: false,
    })
}

fn require_available_selection(
    conn: &Connection,
    plan: &SyncPlan,
    catalog: &SimpleFinAccountSet,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let Err(mut message) = plan.require_selected() else {
        return Ok(());
    };
    let mut state = health::load(conn).map_err(|e| e.to_string())?;
    if account_selection::active_count(conn, Provider::SimpleFin)? > 0 {
        message = "SimpleFIN did not return any of the selected accounts. Saved balances were kept. Review connection health and account choices.".into();
        state.response(catalog, now);
    }
    if !catalog.errors.is_empty() {
        message = format!("{message} {}", catalog.errors.join(" "));
    }
    state.last_error = Some(message.clone());
    health::save(conn, &state)
        .map_err(|e| format!("{message} Could not save connection status: {e}"))?;
    Err(message)
}

#[tauri::command]
pub async fn simplefin_sync(
    db: State<'_, AppDb>,
    sync: State<'_, SimpleFinSyncLock>,
    automatic: Option<bool>,
) -> Result<SimpleFinSyncSummary, String> {
    let _guard = sync.0.lock().await;
    let url = access_url()?;
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut state = health::load(&conn).map_err(|e| e.to_string())?;
        if !state.reserve(chrono::Utc::now(), automatic.unwrap_or(false))? {
            return Ok(SimpleFinSyncSummary {
                accounts_synced: 0,
                holdings_synced: 0,
                transactions_synced: 0,
                accounts_needing_review: 0,
                synced_at: state.last_response_at.clone(),
                warnings: state.messages(),
                skipped: true,
            });
        }
        health::save(&conn, &state).map_err(|e| e.to_string())?;
    }
    let client = SimpleFinClient::new(&url).map_err(|e| record_provider_failure(&db, e))?;
    let catalog = fetch_catalog(&db, &url).await?;
    let (remotes, plan) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        check_connection(&url)?;
        let remotes = review_accounts(&conn, &catalog.accounts)?;
        let plan = account_selection::plan_sync(&conn, Provider::SimpleFin, &remotes)?;
        require_available_selection(&conn, &plan, &catalog, chrono::Utc::now())?;
        (remotes, plan)
    };
    let ids: Vec<&str> = catalog
        .accounts
        .iter()
        .zip(&remotes)
        .filter(|(_, remote)| plan.targets.iter().any(|t| t.remote_id == remote.remote_id))
        .map(|(account, _)| account.id.as_str())
        .collect();
    reserve_request(&db)?;
    let mut account_set = client
        .fetch_accounts(&ids)
        .await
        .map_err(|e| record_provider_failure(&db, e))?;
    for warning in &catalog.errors {
        if !account_set.errors.contains(warning) {
            account_set.errors.push(warning.clone());
        }
    }
    for issue in &catalog.issues {
        if !account_set.issues.contains(issue) {
            account_set.issues.push(issue.clone());
        }
    }
    let now = chrono::Utc::now();
    let result = {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        check_connection(&url)?;
        apply_sync(
            &mut conn,
            &remotes,
            &plan,
            &account_set,
            &now.format("%Y-%m-%d").to_string(),
            &health::stamp(now),
        )
    };
    result.map_err(|message| record_failure(&db, message, false))
}

/// Disconnect SimpleFIN: remove the stored access URL, clear the last-synced marker, and
/// deactivate the connected accounts. Historical snapshots are left untouched.
#[tauri::command]
pub async fn simplefin_disconnect(
    db: State<'_, AppDb>,
    sync: State<'_, SimpleFinSyncLock>,
) -> Result<SimpleFinStatus, String> {
    let _guard = sync.0.lock().await;
    secrets::delete_secret(SIMPLEFIN_ACCESS_URL).map_err(|e| e.to_string())?;
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        delete_setting(&tx, SETTING_LAST_SYNCED).map_err(|e| e.to_string())?;
        account_selection::pause_provider(&tx, Provider::SimpleFin)?;
        let mut state = health::load(&tx).map_err(|e| e.to_string())?;
        state.app_auth_required = false;
        state.last_error = None;
        health::save(&tx, &state).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    simplefin_get_status(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::accounts::aggregated_account_jurisdiction;

    fn select_account(conn: &mut Connection, account: &SimpleFinAccount) {
        let remotes = review_accounts(conn, std::slice::from_ref(account)).unwrap();
        let review = account_selection::discover(conn, Provider::SimpleFin, &remotes).unwrap();
        account_selection::save_choices(
            conn,
            Provider::SimpleFin,
            &remotes,
            &SaveAccountChoices {
                revision: review.revision,
                choices: vec![account_selection::AccountChoice::Create {
                    remote_id: remotes[0].remote_id.clone(),
                }],
            },
        )
        .unwrap();
    }

    fn sync_account(
        conn: &mut Connection,
        account: &SimpleFinAccount,
        today: &str,
        now: &str,
    ) -> i64 {
        let remotes = review_accounts(conn, std::slice::from_ref(account)).unwrap();
        let plan = account_selection::plan_sync(conn, Provider::SimpleFin, &remotes).unwrap();
        apply_sync(
            conn,
            &remotes,
            &plan,
            &SimpleFinAccountSet {
                accounts: vec![account.clone()],
                ..Default::default()
            },
            today,
            now,
        )
        .unwrap();
        plan.targets[0].account_id
    }

    #[test]
    fn maps_account_types_from_name_and_holdings() {
        assert_eq!(map_account_type("TFSA Investment", true), "tfsa");
        assert_eq!(map_account_type("Roth IRA", false), "roth_ira");
        assert_eq!(map_account_type("Everyday Chequing", false), "chequing");
        assert_eq!(map_account_type("High-Interest Savings", false), "savings");
        assert_eq!(map_account_type("Visa Platinum", false), "credit");
        assert_eq!(map_account_type("Self-Directed", true), "brokerage");
        // A plain deposit account with no hints defaults to chequing.
        assert_eq!(map_account_type("My Account", false), "chequing");
    }

    #[test]
    fn jurisdiction_uses_currency_except_for_questrade() {
        assert_eq!(aggregated_account_jurisdiction("CAD", None), "CA");
        assert_eq!(aggregated_account_jurisdiction("cad", None), "CA");
        assert_eq!(aggregated_account_jurisdiction("USD", None), "US");
        assert_eq!(
            aggregated_account_jurisdiction("USD", Some("Questrade")),
            "CA"
        );
    }

    #[test]
    fn holding_unit_prices_divide_totals_and_guard_zero_shares() {
        let h = SimpleFinHolding {
            symbol: "AAPL".into(),
            shares: 10.0,
            market_value: Some(1500.0),
            cost_basis: Some(1000.0),
            currency: Some("USD".into()),
        };
        assert_eq!(holding_unit_prices(&h), (Some(150.0), Some(100.0)));

        let zero = SimpleFinHolding {
            symbol: "ZERO".into(),
            shares: 0.0,
            market_value: Some(50.0),
            cost_basis: Some(40.0),
            currency: None,
        };
        assert_eq!(holding_unit_prices(&zero), (None, None));
    }

    fn brokerage_account() -> SimpleFinAccount {
        SimpleFinAccount {
            id: "act-1".into(),
            name: "Self-Directed Brokerage".into(),
            number: None,
            currency: "USD".into(),
            balance: Some(1000.0),
            balance_date: Some(1_735_689_600),
            institution: Some("Wealthsimple".into()),
            connection_id: None,
            holdings_reported: true,
            holdings: vec![SimpleFinHolding {
                symbol: "AAPL".into(),
                shares: 10.0,
                market_value: Some(1500.0),
                cost_basis: Some(1000.0),
                currency: Some("USD".into()),
            }],
            transactions: vec![],
        }
    }

    #[test]
    fn reconcile_inserts_then_updates_by_connector_ref() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        let account = brokerage_account();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        let holdings = replace_holdings(
            &conn,
            id,
            &account,
            &account.currency,
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(holdings, 1);

        // The account is created with the brokerage type and SimpleFIN connector metadata.
        let (kind, account_type): (String, String) = conn
            .query_row(
                "SELECT connector_kind, account_type FROM accounts WHERE connector_ref = 'act-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "simplefin");
        assert_eq!(account_type, "brokerage");

        // Balance snapshot + derived per-share holding figures land in the schema.
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 1000.0);
        let (qty, last_price, avg_cost): (f64, f64, f64) = conn
            .query_row(
                "SELECT quantity, last_price, average_cost FROM holdings WHERE account_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((qty, last_price, avg_cost), (10.0, 150.0, 100.0));

        // A second sync updates the same row (no duplicate) and replaces holdings.
        let mut updated = brokerage_account();
        updated.balance = Some(1200.0);
        updated.holdings.clear();
        let id2 = sync_account(&mut conn, &updated, "2025-01-01", "2025-01-02T00:00:00Z");
        let holdings2 = replace_holdings(
            &conn,
            id2,
            &updated,
            &updated.currency,
            "2025-01-02T00:00:00Z",
        )
        .unwrap();
        assert_eq!(id2, id);
        assert_eq!(holdings2, 0);

        let account_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(account_rows, 1);
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(balance, 1200.0);
        let holding_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM holdings WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(holding_rows, 0);
    }

    fn account_with_transactions() -> SimpleFinAccount {
        SimpleFinAccount {
            id: "act-9".into(),
            name: "Everyday Chequing".into(),
            number: None,
            currency: "CAD".into(),
            balance: Some(500.0),
            balance_date: Some(1_735_689_600),
            institution: Some("Scotiabank".into()),
            connection_id: None,
            holdings_reported: true,
            holdings: vec![],
            transactions: vec![
                SimpleFinTransaction {
                    id: "txn-1".into(),
                    amount: -42.50,
                    description: "GROCERY STORE".into(),
                    posted: Some(1_700_000_000),
                    memo: None,
                },
                SimpleFinTransaction {
                    id: "txn-2".into(),
                    amount: 2500.0,
                    description: "MICROSOFT PAYROLL".into(),
                    posted: None,
                    memo: Some("bi-weekly".into()),
                },
            ],
        }
    }

    #[test]
    fn epoch_to_date_converts_or_falls_back() {
        assert_eq!(
            epoch_to_date(Some(1_700_000_000), "2025-01-01"),
            "2023-11-14"
        );
        assert_eq!(epoch_to_date(None, "2025-01-01"), "2025-01-01");
    }

    #[test]
    fn snapshot_date_requires_a_valid_source_date() {
        // A real (past) balance-date is used as-is — that is what keeps a stale account honest.
        assert_eq!(
            snapshot_date_for(Some(1_700_000_000), "2025-07-01").as_deref(),
            Some("2023-11-14")
        );
        assert!(snapshot_date_for(None, "2025-07-01").is_none());
        assert!(snapshot_date_for(Some(4_102_444_800), "2025-07-01").is_none());
        assert!(snapshot_date_for(Some(0), "2025-07-01").is_none());
    }

    fn stale_credit_card(balance_date: Option<i64>) -> SimpleFinAccount {
        SimpleFinAccount {
            id: "cc-scotia".into(),
            name: "Scotiabank Visa".into(),
            number: None,
            currency: "CAD".into(),
            balance: Some(-1234.56),
            balance_date,
            institution: Some("Scotiabank".into()),
            connection_id: None,
            holdings_reported: true,
            holdings: vec![],
            transactions: vec![],
        }
    }

    #[test]
    fn files_balance_snapshot_under_simplefin_balance_date() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        // SimpleFIN reports the card's balance as of 2023-11-14 (epoch 1_700_000_000).
        let card = stale_credit_card(Some(1_700_000_000));
        select_account(&mut conn, &card);
        let id = sync_account(&mut conn, &card, "2025-07-01", "2025-07-01T00:00:00Z");

        // The snapshot is filed under the reported balance-date, not "today".
        let (date, balance): (String, f64) = conn
            .query_row(
                "SELECT snapshot_date, balance FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(date, "2023-11-14");
        assert_eq!(balance, -1234.56);

        // Re-syncing days later with the SAME stale balance-date rewrites that same dated row —
        // it must NOT mint a fresh "today" snapshot, which is what used to hide the stall and make
        // the balance look current while it hadn't actually moved in weeks.
        sync_account(&mut conn, &card, "2025-07-05", "2025-07-05T00:00:00Z");
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        let latest: String = conn
            .query_row(
                "SELECT snapshot_date FROM balance_snapshots \
                 WHERE account_id = ?1 ORDER BY snapshot_date DESC LIMIT 1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(latest, "2023-11-14");
    }

    #[test]
    fn balance_snapshot_without_balance_date_does_not_look_fresh() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        let card = stale_credit_card(None);
        select_account(&mut conn, &card);
        let id = sync_account(&mut conn, &card, "2025-07-01", "2025-07-01T00:00:00Z");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM balance_snapshots WHERE account_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn scoped_ids_migrate_existing_accounts_without_losing_history_or_currency_overrides() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let mut account = brokerage_account();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        conn.execute("UPDATE accounts SET currency = 'JMD' WHERE id = ?1", [id])
            .unwrap();
        account.connection_id = Some("personal".into());
        account.holdings_reported = false;
        account.holdings.clear();
        let migrated = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        assert_eq!(migrated, id);
        let (currency, kind): (String, String) = conn
            .query_row(
                "SELECT currency, account_type FROM accounts WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(currency, "JMD");
        assert_eq!(kind, "brokerage");
        account.connection_id = Some("joint".into());
        let remotes = review_accounts(&conn, std::slice::from_ref(&account)).unwrap();
        assert!(
            account_selection::plan_sync(&conn, Provider::SimpleFin, &remotes)
                .unwrap()
                .targets
                .is_empty()
        );
        select_account(&mut conn, &account);
        assert_ne!(
            sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z"),
            id
        );
    }

    #[test]
    fn health_stays_stale_after_a_cached_sync_and_missing_accounts_keep_their_balance() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let checked = chrono::DateTime::parse_from_rfc3339("2025-01-03T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let account = brokerage_account();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-03", &health::stamp(checked));
        let mut state = health::load(&conn).unwrap();
        let data = SimpleFinAccountSet {
            accounts: vec![account.clone()],
            ..Default::default()
        };
        state.response(&data, checked);
        state.record_account(id, &account, checked);
        health::save(&conn, &state).unwrap();
        assert_eq!(
            state
                .connections(&conn, true, checked - chrono::Duration::seconds(1))
                .unwrap()[0]
                .status,
            super::health::BalanceStatus::Current
        );
        assert_eq!(
            state.connections(&conn, true, checked).unwrap()[0].status,
            super::health::BalanceStatus::Stale
        );
        let mut older = account.clone();
        older.balance = Some(99_999.0);
        older.balance_date = Some(1_735_689_599);
        sync_account(&mut conn, &older, "2025-01-03", &health::stamp(checked));
        let balance: f64 = conn
            .query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(balance, 1000.0);
        state.response(
            &SimpleFinAccountSet::default(),
            checked + chrono::Duration::hours(6),
        );
        assert_eq!(
            state.connections(&conn, true, checked).unwrap()[0].status,
            super::health::BalanceStatus::Missing
        );
    }

    #[test]
    fn reconcile_transactions_dedups_and_preserves_user_tags() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        let account = account_with_transactions();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        let n = reconcile_transactions(
            &conn,
            id,
            &account.transactions,
            &account.currency,
            "2025-01-01",
        )
        .unwrap();
        assert_eq!(n, 2);

        // The missing-posted transaction falls back to today's date; the dated one is converted.
        let date: String = conn
            .query_row(
                "SELECT txn_date FROM transactions WHERE connector_ref = 'txn-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(date, "2023-11-14");

        // The user manually retags the payroll row.
        conn.execute(
            "UPDATE transactions SET flow_override = 'income' WHERE connector_ref = 'txn-2'",
            [],
        )
        .unwrap();

        // A second sync updates amounts in place (no duplicates) and keeps the override.
        let mut updated = account_with_transactions();
        updated.transactions[0].amount = -50.0;
        let n2 = reconcile_transactions(
            &conn,
            id,
            &updated.transactions,
            &updated.currency,
            "2025-01-02",
        )
        .unwrap();
        assert_eq!(n2, 2);

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
        let amount: f64 = conn
            .query_row(
                "SELECT amount FROM transactions WHERE connector_ref = 'txn-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(amount, -50.0);
        let override_kept: Option<String> = conn
            .query_row(
                "SELECT flow_override FROM transactions WHERE connector_ref = 'txn-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(override_kept.as_deref(), Some("income"));
    }

    #[test]
    fn reconcile_preserves_a_user_corrected_currency() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        // First sync: SimpleFIN mislabels this Jamaican account as CAD.
        let mut account = brokerage_account();
        account.id = "act-jm".into();
        account.currency = "CAD".into();
        account.balance = Some(75000.0);
        account.holdings.clear();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");

        // The user corrects it to JMD (as `update_account_currency` would).
        conn.execute(
            "UPDATE accounts SET currency = 'JMD' WHERE id = ?1",
            params![id],
        )
        .unwrap();

        // A later sync still reports CAD, but the correction must stick and the new snapshot
        // must inherit the stored JMD currency.
        account.balance_date = Some(1_735_776_000);
        let id2 = sync_account(&mut conn, &account, "2025-01-02", "2025-01-02T00:00:00Z");
        assert_eq!(id2, id);

        let currency: String = conn
            .query_row(
                "SELECT currency FROM accounts WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(currency, "JMD");

        let snapshot_currency: String = conn
            .query_row(
                "SELECT currency FROM balance_snapshots WHERE account_id = ?1 AND snapshot_date = '2025-01-02'",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_currency, "JMD");
    }

    #[test]
    fn settings_roundtrip_and_delete() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();

        assert_eq!(get_setting(&conn, SETTING_LAST_SYNCED).unwrap(), None);
        set_setting(&conn, SETTING_LAST_SYNCED, "2025-01-01T00:00:00Z").unwrap();
        assert_eq!(
            get_setting(&conn, SETTING_LAST_SYNCED).unwrap().as_deref(),
            Some("2025-01-01T00:00:00Z")
        );
        delete_setting(&conn, SETTING_LAST_SYNCED).unwrap();
        assert_eq!(get_setting(&conn, SETTING_LAST_SYNCED).unwrap(), None);
    }

    #[test]
    fn scoped_account_ids_select_independently_and_keep_exclusions() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let mut personal = brokerage_account();
        personal.connection_id = Some("personal".into());
        let mut joint = personal.clone();
        joint.connection_id = Some("joint".into());
        let accounts = vec![personal.clone(), joint.clone()];
        let remotes = review_accounts(&conn, &accounts).unwrap();
        let revision = account_selection::discover(&conn, Provider::SimpleFin, &remotes)
            .unwrap()
            .revision;
        account_selection::save_choices(
            &mut conn,
            Provider::SimpleFin,
            &remotes,
            &SaveAccountChoices {
                revision,
                choices: vec![
                    account_selection::AccountChoice::Create {
                        remote_id: personal.reference(),
                    },
                    account_selection::AccountChoice::Ignore {
                        remote_id: joint.reference(),
                    },
                ],
            },
        )
        .unwrap();
        let plan = account_selection::plan_sync(&conn, Provider::SimpleFin, &remotes).unwrap();
        assert_eq!(plan.targets.len(), 1);
        let result = apply_sync(
            &mut conn,
            &remotes,
            &plan,
            &SimpleFinAccountSet {
                accounts,
                ..Default::default()
            },
            "2025-01-01",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(result.accounts_synced, 1);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(plan.targets[0].remote_id, personal.reference());
    }

    #[test]
    fn legacy_ignore_is_not_bypassed_by_scoped_ids_and_ambiguous_ids_are_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let mut account = brokerage_account();
        select_account(&mut conn, &account);
        let id = sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        account_selection::deactivate_account(&mut conn, id).unwrap();
        account.connection_id = Some("personal".into());
        let remotes = review_accounts(&conn, std::slice::from_ref(&account)).unwrap();
        assert!(
            account_selection::plan_sync(&conn, Provider::SimpleFin, &remotes)
                .unwrap()
                .targets
                .is_empty()
        );
        let mut joint = account.clone();
        joint.connection_id = Some("joint".into());
        assert!(review_accounts(&conn, &[account, joint])
            .unwrap_err()
            .contains("multiple bank"));
    }

    #[test]
    fn failed_connection_preserves_its_balance_and_holdings_without_poisoning_other_accounts() {
        use crate::connector::simplefin::SimpleFinIssue;
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let mut first = brokerage_account();
        first.connection_id = Some("a".into());
        let mut second = first.clone();
        second.connection_id = Some("b".into());
        select_account(&mut conn, &first);
        select_account(&mut conn, &second);
        let id = sync_account(&mut conn, &first, "2025-01-01", "2025-01-01T00:00:00Z");
        let id2 = sync_account(&mut conn, &second, "2025-01-01", "2025-01-01T00:00:00Z");
        first.balance = Some(99999.0);
        first.holdings.clear();
        second.balance = Some(2000.0);
        second.holdings_reported = false;
        second.holdings.clear();
        let data = SimpleFinAccountSet {
            accounts: vec![first, second],
            errors: vec!["Approve bank A".into()],
            issues: vec![SimpleFinIssue {
                code: "con.auth".into(),
                msg: "Approve bank A".into(),
                conn_id: Some("a".into()),
                account_id: None,
            }],
            ..Default::default()
        };
        let remotes = review_accounts(&conn, &data.accounts).unwrap();
        let plan = account_selection::plan_sync(&conn, Provider::SimpleFin, &remotes).unwrap();
        let summary = apply_sync(
            &mut conn,
            &remotes,
            &plan,
            &data,
            "2025-01-01",
            "2025-01-01T01:00:00Z",
        )
        .unwrap();
        assert_eq!(summary.holdings_synced, 0);
        assert_eq!(summary.warnings, ["Approve bank A"]);
        let balance = |id| {
            conn.query_row(
                "SELECT balance FROM balance_snapshots WHERE account_id = ?1",
                [id],
                |r| r.get::<_, f64>(0),
            )
            .unwrap()
        };
        assert_eq!(balance(id), 1000.0);
        assert_eq!(balance(id2), 2000.0);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM holdings", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let now = chrono::DateTime::parse_from_rfc3339("2025-01-01T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let banks = health::load(&conn)
            .unwrap()
            .connections(&conn, true, now)
            .unwrap();
        assert_eq!(
            banks.iter().find(|b| b.id == "a").unwrap().status,
            health::BalanceStatus::ReauthRequired
        );
        assert_eq!(
            banks.iter().find(|b| b.id == "b").unwrap().status,
            health::BalanceStatus::Current
        );
        account_selection::deactivate_account(&mut conn, id).unwrap();
        let banks = health::load(&conn)
            .unwrap()
            .connections(&conn, true, now)
            .unwrap();
        assert_eq!(banks.len(), 1);
        assert_eq!(banks[0].id, "b");
    }

    #[test]
    fn missing_all_selected_accounts_records_missing_health_without_successful_sync() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let account = brokerage_account();
        select_account(&mut conn, &account);
        sync_account(&mut conn, &account, "2025-01-01", "2025-01-01T00:00:00Z");
        let now = chrono::DateTime::parse_from_rfc3339("2025-01-01T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let plan = account_selection::plan_sync(&conn, Provider::SimpleFin, &[]).unwrap();
        assert!(
            require_available_selection(&conn, &plan, &SimpleFinAccountSet::default(), now)
                .unwrap_err()
                .contains("did not return any")
        );
        assert_eq!(
            get_setting(&conn, SETTING_LAST_SYNCED).unwrap().as_deref(),
            Some("2025-01-01T00:00:00Z")
        );
        assert_eq!(
            health::load(&conn)
                .unwrap()
                .connections(&conn, true, now)
                .unwrap()[0]
                .status,
            health::BalanceStatus::Missing
        );
    }
}
