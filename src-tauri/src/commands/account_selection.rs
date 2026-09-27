//! Shared, explicit ownership of aggregator accounts. Discovery is read-only; only a reviewed
//! choice can create/revive an account or switch its source. Old bindings remain as tombstones.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::accounts::aggregated_account_jurisdiction;

const REVISION_SETTING: &str = "account_selection_revision";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    SnapTrade,
    SimpleFin,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SnapTrade => "snaptrade",
            Self::SimpleFin => "simplefin",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoteAccount {
    pub remote_id: String,
    pub connection_id: Option<String>,
    pub name: String,
    pub institution: String,
    pub account_type: String,
    pub currency: Option<String>,
    pub masked_number: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalAccount {
    pub id: i64,
    pub name: String,
    pub institution: String,
    pub account_type: String,
    pub currency: String,
    pub connector_kind: String,
    pub connector_ref: Option<String>,
    pub is_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionState {
    Unreviewed,
    Sync,
    Ignore,
}

#[derive(Debug, Serialize)]
pub struct DiscoveredAccount {
    #[serde(flatten)]
    pub remote: RemoteAccount,
    pub selection: SelectionState,
    pub local_account_id: Option<i64>,
    pub can_resume: bool,
    pub linkable_account_ids: Vec<i64>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AccountReview {
    pub revision: String,
    pub accounts: Vec<DiscoveredAccount>,
    pub local_accounts: Vec<LocalAccount>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountChoice {
    Create { remote_id: String },
    Link { remote_id: String, account_id: i64 },
    Sync { remote_id: String },
    Ignore { remote_id: String },
}

impl AccountChoice {
    fn remote_id(&self) -> &str {
        match self {
            Self::Create { remote_id }
            | Self::Link { remote_id, .. }
            | Self::Sync { remote_id }
            | Self::Ignore { remote_id } => remote_id,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAccountChoices {
    pub revision: String,
    pub choices: Vec<AccountChoice>,
}

#[derive(Debug, Serialize)]
struct Binding {
    provider: String,
    remote_id: String,
    account_id: Option<i64>,
    decision: String,
    institution: Option<String>,
    connection_id: Option<String>,
}

#[derive(Debug)]
pub struct SyncTarget {
    pub remote_id: String,
    pub account_id: i64,
    pub currency: String,
}

pub struct SyncPlan {
    pub revision: String,
    pub targets: Vec<SyncTarget>,
    pub accounts_needing_review: usize,
}

impl SyncPlan {
    pub fn require_selected(&self) -> Result<(), String> {
        if self.targets.is_empty() {
            Err(format!(
                "No accounts are selected for sync. Open Choose accounts to sync to select or \
                 resume accounts. {} new account(s) need review.",
                self.accounts_needing_review
            ))
        } else {
            Ok(())
        }
    }
}

pub fn mask_account_number(number: Option<&str>) -> Option<String> {
    let number: String = number?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if number.is_empty() {
        return None;
    }
    let suffix = &number[number.len().saturating_sub(4)..];
    Some(format!("****{suffix}"))
}

fn valid_currency(currency: Option<&str>) -> bool {
    currency.is_some_and(|code| code.len() == 3 && code.bytes().all(|c| c.is_ascii_alphabetic()))
}

fn owns(account: &LocalAccount, provider: Provider, remote_id: &str) -> bool {
    account.connector_kind == provider.as_str()
        && account.connector_ref.as_deref() == Some(remote_id)
}

fn read_local_accounts(conn: &Connection) -> Result<Vec<LocalAccount>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, institution, account_type, currency, connector_kind, connector_ref, \
             is_active FROM accounts ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LocalAccount {
                id: r.get(0)?,
                name: r.get(1)?,
                institution: r.get(2)?,
                account_type: r.get(3)?,
                currency: r.get(4)?,
                connector_kind: r.get(5)?,
                connector_ref: r.get(6)?,
                is_active: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn read_bindings(conn: &Connection) -> Result<Vec<Binding>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT provider, remote_id, account_id, decision, institution, connection_id FROM account_sync_selections \
             ORDER BY provider, remote_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Binding {
                provider: r.get(0)?,
                remote_id: r.get(1)?,
                account_id: r.get(2)?,
                decision: r.get(3)?,
                institution: r.get(4)?,
                connection_id: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn validate_target(
    provider: Provider,
    remote: &RemoteAccount,
    target: &LocalAccount,
    bound_id: Option<i64>,
    bindings: &[Binding],
) -> Result<(), String> {
    if bound_id.is_some_and(|id| id != target.id) {
        return Err(
            "This provider account already has an imported account. Moving or merging its \
             history is not supported. Choose Ignore to hide a duplicate without deleting \
             history; its original account will be untouched."
                .into(),
        );
    }
    let current_owner = owns(target, provider, &remote.remote_id);
    if !target.is_active && !(bound_id == Some(target.id) && current_owner) {
        return Err("The target account is hidden or deleted. Refresh the account review.".into());
    }
    if !matches!(
        target.connector_kind.as_str(),
        "manual" | "simplefin" | "snaptrade"
    ) {
        return Err(
            "Only manual, SimpleFIN, or SnapTrade accounts support explicit linking.".into(),
        );
    }
    if target.connector_kind == provider.as_str() && !current_owner {
        return Err(
            "The target is already assigned to a different account from this provider.".into(),
        );
    }
    if bindings.iter().any(|b| {
        b.provider == provider.as_str()
            && b.account_id == Some(target.id)
            && b.remote_id != remote.remote_id
    }) {
        return Err("The target has history from a different account ID at this provider. Merging those histories is not supported.".into());
    }
    if matches!(target.connector_kind.as_str(), "snaptrade" | "simplefin")
        && target.connector_ref.is_none()
    {
        return Err(
            "The target has no provider ID, so its old source cannot be safely excluded.".into(),
        );
    }
    // An existing owner can keep a user-corrected currency/type. A new source must match.
    if !current_owner {
        if !valid_currency(remote.currency.as_deref())
            || !remote
                .currency
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(&target.currency))
        {
            return Err("The provider and target accounts must have the same currency.".into());
        }
        if remote.account_type != target.account_type {
            return Err("The provider and target accounts must have the same account type.".into());
        }
    }
    Ok(())
}

/// Combines a provider's metadata with local choices without writing any database rows.
pub fn discover(
    conn: &Connection,
    provider: Provider,
    remotes: &[RemoteAccount],
) -> Result<AccountReview, String> {
    let local_accounts = read_local_accounts(conn)?;
    let bindings = read_bindings(conn)?;
    let generation: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [REVISION_SETTING],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let direct_questrade_active = local_accounts
        .iter()
        .any(|a| a.is_active && a.connector_kind == "questrade");
    let mut sorted = remotes.to_vec();
    sorted.sort_by(|a, b| a.remote_id.cmp(&b.remote_id));
    let mut ids = HashSet::new();
    let mut accounts = Vec::with_capacity(sorted.len());
    for remote in &sorted {
        if remote.remote_id.trim().is_empty() || !ids.insert(&remote.remote_id) {
            return Err(
                "The provider returned missing or duplicate account IDs. Nothing was imported."
                    .into(),
            );
        }
        let binding = bindings
            .iter()
            .find(|b| b.provider == provider.as_str() && b.remote_id == remote.remote_id);
        let local = binding
            .and_then(|b| b.account_id)
            .map(|id| {
                local_accounts.iter().find(|a| a.id == id).ok_or_else(|| {
                    "A saved account selection points to a missing local account. Nothing was imported."
                        .to_string()
                })
            })
            .transpose()?;
        if binding.is_none()
            && local_accounts
                .iter()
                .any(|a| owns(a, provider, &remote.remote_id))
        {
            return Err(
                "An imported provider ID has no unambiguous saved selection. Restart TrueNorth \
                 to migrate existing accounts; duplicate provider IDs require manual repair."
                    .into(),
            );
        }
        if binding.is_some_and(|b| b.decision == "sync")
            && !local.is_some_and(|a| owns(a, provider, &remote.remote_id))
        {
            return Err(
                "A saved sync selection no longer owns its local account. Refresh account choices."
                    .into(),
            );
        }
        let questrade_hidden = direct_questrade_active
            && (remote
                .institution
                .to_ascii_lowercase()
                .contains("questrade")
                || local.is_some_and(|a| a.institution.to_ascii_lowercase().contains("questrade"))
                || binding
                    .and_then(|b| b.institution.as_deref())
                    .is_some_and(|name| name.to_ascii_lowercase().contains("questrade")));
        let selection = match binding {
            None => SelectionState::Unreviewed,
            Some(b)
                if b.decision == "sync"
                    && local.is_some_and(|a| a.is_active)
                    && !questrade_hidden =>
            {
                SelectionState::Sync
            }
            Some(_) => SelectionState::Ignore,
        };
        let unavailable_reason = if questrade_hidden {
            Some("Direct Questrade is active. Aggregated Questrade accounts stay hidden to avoid double-counting.".into())
        } else if let Some(target) = local {
            validate_target(provider, remote, target, Some(target.id), &bindings).err()
        } else if !valid_currency(remote.currency.as_deref()) {
            Some("The provider has not reported a supported 3-letter currency. Creation and linking are unavailable.".into())
        } else {
            None
        };
        let can_resume = local.is_some() && unavailable_reason.is_none();
        let linkable_account_ids = if local.is_none() && unavailable_reason.is_none() {
            local_accounts
                .iter()
                .filter(|a| validate_target(provider, remote, a, None, &bindings).is_ok())
                .map(|a| a.id)
                .collect()
        } else {
            Vec::new()
        };
        accounts.push(DiscoveredAccount {
            remote: remote.clone(),
            selection,
            local_account_id: local.map(|a| a.id),
            can_resume,
            linkable_account_ids,
            unavailable_reason,
        });
    }
    // No balances in this token: changing financial data must not invalidate an account review.
    // Metadata, bindings and a monotonic generation protect against stale saves and in-flight syncs.
    let state = serde_json::to_vec(&(provider, sorted, &local_accounts, bindings, generation))
        .map_err(|e| e.to_string())?;
    Ok(AccountReview {
        revision: hex::encode(Sha256::digest(state)),
        accounts,
        local_accounts,
        warnings: Vec::new(),
    })
}

fn write_binding(
    conn: &Connection,
    provider: &str,
    remote_id: &str,
    account_id: Option<i64>,
    decision: &str,
    institution: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO account_sync_selections (provider, remote_id, account_id, decision, institution) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(provider, remote_id) DO UPDATE SET \
         account_id = excluded.account_id, decision = excluded.decision, \
         institution = COALESCE(excluded.institution, account_sync_selections.institution)",
        params![provider, remote_id, account_id, decision, institution],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn bump_revision(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1') \
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [REVISION_SETTING],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn assign(
    conn: &Connection,
    provider: Provider,
    remote: &RemoteAccount,
    target: &LocalAccount,
) -> Result<(), String> {
    if matches!(target.connector_kind.as_str(), "snaptrade" | "simplefin") {
        let old_ref = target.connector_ref.as_deref().ok_or(
            "The target account has no provider ID. Linking cannot safely replace its source.",
        )?;
        write_binding(
            conn,
            &target.connector_kind,
            old_ref,
            Some(target.id),
            "ignore",
            None,
        )?;
    }
    // Retain all prior bindings as exclusions, even after switching back to an earlier provider.
    conn.execute(
        "UPDATE account_sync_selections SET decision = 'ignore' WHERE account_id = ?1",
        [target.id],
    )
    .map_err(|e| e.to_string())?;
    write_binding(
        conn,
        provider.as_str(),
        &remote.remote_id,
        Some(target.id),
        "sync",
        Some(&remote.institution),
    )?;
    conn.execute(
        "UPDATE accounts SET connector_kind = ?1, connector_ref = ?2, is_active = 1, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?3",
        params![provider.as_str(), remote.remote_id, target.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Validate the entire reviewed batch against fresh provider metadata and local state, then
/// commit it atomically. Never accept account metadata or eligibility supplied by the frontend.
pub fn save_choices(
    conn: &mut Connection,
    provider: Provider,
    remotes: &[RemoteAccount],
    payload: &SaveAccountChoices,
) -> Result<AccountReview, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let review = discover(&tx, provider, remotes)?;
    let bindings = read_bindings(&tx)?;
    if payload.revision != review.revision {
        return Err("Account choices or provider details changed. Reload Choose accounts and review again; nothing was saved.".into());
    }
    if payload.choices.is_empty() {
        return Err("Choose at least one account action before saving.".into());
    }
    let mut remote_ids = HashSet::new();
    let mut target_ids = HashSet::new();
    for choice in &payload.choices {
        if !remote_ids.insert(choice.remote_id()) {
            return Err("An account was submitted more than once. Nothing was saved.".into());
        }
        let row = review
            .accounts
            .iter()
            .find(|a| a.remote.remote_id == choice.remote_id())
            .ok_or(
                "A selected provider account is no longer available. Reload the account review.",
            )?;
        if !matches!(choice, AccountChoice::Ignore { .. }) {
            if let Some(reason) = &row.unavailable_reason {
                return Err(reason.clone());
            }
        }
        let target_id = match choice {
            AccountChoice::Create { .. } => {
                if row.local_account_id.is_some() {
                    return Err("This account was already imported. Resume its existing row instead of creating a duplicate.".into());
                }
                None
            }
            AccountChoice::Sync { .. } => Some(row.local_account_id.ok_or(
                "This account has not been imported. Choose Create new or Link to existing.",
            )?),
            AccountChoice::Link { account_id, .. } => Some(*account_id),
            AccountChoice::Ignore { .. } => None,
        };
        if let Some(id) = target_id {
            if !target_ids.insert(id) {
                return Err("Two provider accounts cannot sync into the same local account. Nothing was saved.".into());
            }
            let target = review
                .local_accounts
                .iter()
                .find(|a| a.id == id)
                .ok_or("The target account no longer exists. Reload the account review.")?;
            validate_target(
                provider,
                &row.remote,
                target,
                row.local_account_id,
                &bindings,
            )?;
        }
    }
    for choice in &payload.choices {
        let row = review
            .accounts
            .iter()
            .find(|a| a.remote.remote_id == choice.remote_id())
            .ok_or("The reviewed account is missing.")?;
        match choice {
            AccountChoice::Ignore { remote_id } => {
                write_binding(
                    &tx,
                    provider.as_str(),
                    remote_id,
                    row.local_account_id,
                    "ignore",
                    Some(&row.remote.institution),
                )?;
                // An old provider's Ignore must never hide the account now owned by its replacement.
                tx.execute(
                    "UPDATE accounts SET is_active = 0, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
                     WHERE id = ?1 AND connector_kind = ?2 AND connector_ref = ?3",
                    params![row.local_account_id, provider.as_str(), remote_id],
                )
                .map_err(|e| e.to_string())?;
            }
            AccountChoice::Create { remote_id } => {
                let currency = row
                    .remote
                    .currency
                    .as_deref()
                    .ok_or("Account currency is missing.")?;
                let name = match &row.remote.masked_number {
                    Some(number) => format!("{} ({number})", row.remote.name),
                    None => row.remote.name.clone(),
                };
                tx.execute(
                    "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, \
                     connector_kind, connector_ref) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        name, row.remote.institution, row.remote.account_type, currency,
                        aggregated_account_jurisdiction(currency, Some(&row.remote.institution)),
                        provider.as_str(), remote_id
                    ],
                )
                .map_err(|e| e.to_string())?;
                write_binding(
                    &tx,
                    provider.as_str(),
                    remote_id,
                    Some(tx.last_insert_rowid()),
                    "sync",
                    Some(&row.remote.institution),
                )?;
            }
            AccountChoice::Link { account_id, .. } => {
                let target = review
                    .local_accounts
                    .iter()
                    .find(|a| a.id == *account_id)
                    .ok_or("The target account no longer exists.")?;
                assign(&tx, provider, &row.remote, target)?;
            }
            AccountChoice::Sync { .. } => {
                let target = review
                    .local_accounts
                    .iter()
                    .find(|a| Some(a.id) == row.local_account_id)
                    .ok_or("The imported account no longer exists.")?;
                assign(&tx, provider, &row.remote, target)?;
            }
        }
        tx.execute(
            "UPDATE account_sync_selections SET connection_id = ?1 \
             WHERE provider = ?2 AND remote_id = ?3 AND connection_id IS NULL",
            params![
                row.remote.connection_id,
                provider.as_str(),
                row.remote.remote_id
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    bump_revision(&tx)?;
    let result = discover(&tx, provider, remotes)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

pub fn plan_sync(
    conn: &Connection,
    provider: Provider,
    remotes: &[RemoteAccount],
) -> Result<SyncPlan, String> {
    let review = discover(conn, provider, remotes)?;
    let mut targets = Vec::new();
    for row in &review.accounts {
        if row.selection != SelectionState::Sync {
            continue;
        }
        let account = review
            .local_accounts
            .iter()
            .find(|a| Some(a.id) == row.local_account_id)
            .ok_or("A selected account no longer exists.")?;
        targets.push(SyncTarget {
            remote_id: row.remote.remote_id.clone(),
            account_id: account.id,
            currency: account.currency.clone(),
        });
    }
    Ok(SyncPlan {
        revision: review.revision,
        targets,
        accounts_needing_review: review
            .accounts
            .iter()
            .filter(|a| a.selection == SelectionState::Unreviewed)
            .count(),
    })
}

pub fn validate_sync(
    conn: &Connection,
    provider: Provider,
    remotes: &[RemoteAccount],
    plan: &SyncPlan,
) -> Result<(), String> {
    if discover(conn, provider, remotes)?.revision != plan.revision {
        return Err("Account choices changed while syncing. Nothing was imported; sync again with the saved choices.".into());
    }
    Ok(())
}

pub fn active_count(conn: &Connection, provider: Provider) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM account_sync_selections s JOIN accounts a ON a.id = s.account_id \
         WHERE s.provider = ?1 AND s.decision = 'sync' AND a.is_active = 1 \
         AND a.connector_kind = s.provider AND a.connector_ref = s.remote_id",
        [provider.as_str()],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Call inside a transaction, both on disconnect and when changing the provider's credentials.
pub fn pause_provider(conn: &Connection, provider: Provider) -> Result<(), String> {
    conn.execute(
        "UPDATE account_sync_selections SET decision = 'ignore' WHERE provider = ?1",
        [provider.as_str()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE connector_kind = ?1",
        [provider.as_str()],
    )
    .map_err(|e| e.to_string())?;
    bump_revision(conn)
}

pub fn deactivate_account(conn: &mut Connection, account_id: i64) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let affected = tx.execute(
        "UPDATE accounts SET is_active = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         WHERE id = ?1",
        [account_id],
    )
    .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err(format!("Account {account_id} not found."));
    }
    tx.execute(
        "UPDATE account_sync_selections SET decision = 'ignore' WHERE account_id = ?1",
        [account_id],
    )
    .map_err(|e| e.to_string())?;
    bump_revision(&tx)?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{simplefin, snaptrade};
    use crate::connector::simplefin::{
        SimpleFinAccount, SimpleFinAccountSet, SimpleFinHolding, SimpleFinTransaction,
    };
    use crate::connector::snaptrade::{SnapAccount, SnapPosition};
    use crate::db::{apply_schema, reconcile_aggregated_questrade_accounts};

    const TODAY: &str = "2026-09-27";
    const NOW: &str = "2026-09-27T12:00:00Z";

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        conn
    }

    fn snap_accounts() -> Vec<(SnapAccount, Vec<SnapPosition>)> {
        [("snap-1", "12340580"), ("snap-2", "98767986")]
            .into_iter()
            .map(|(id, number)| {
                (
                    SnapAccount {
                        id: id.into(),
                        name: Some("Individual".into()),
                        number: Some(number.into()),
                        institution_name: Some("Robinhood".into()),
                        raw_type: Some("Individual".into()),
                        balance_total: Some(1000.0),
                        currency: Some("USD".into()),
                    },
                    vec![SnapPosition {
                        symbol: "SYNTH".into(),
                        units: 10.0,
                        price: Some(100.0),
                        average_purchase_price: Some(80.0),
                        currency: Some("USD".into()),
                    }],
                )
            })
            .collect()
    }

    fn snap_remotes(data: &[(SnapAccount, Vec<SnapPosition>)]) -> Vec<RemoteAccount> {
        snaptrade::remote_accounts(&data.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>())
    }

    fn simplefin_accounts() -> SimpleFinAccountSet {
        SimpleFinAccountSet {
            accounts: ["sf-1", "sf-2"]
                .into_iter()
                .map(|id| SimpleFinAccount {
                    id: id.into(),
                    name: "Individual Brokerage".into(),
                    number: None,
                    institution: Some("Robinhood".into()),
                    connection_id: None,
                    holdings_reported: true,
                    currency: "USD".into(),
                    balance: Some(1000.0),
                    balance_date: Some(
                        chrono::DateTime::parse_from_rfc3339(NOW)
                            .unwrap()
                            .timestamp(),
                    ),
                    holdings: vec![SimpleFinHolding {
                        symbol: "SYNTH".into(),
                        shares: 10.0,
                        market_value: Some(1000.0),
                        cost_basis: Some(800.0),
                        currency: Some("USD".into()),
                    }],
                    transactions: vec![SimpleFinTransaction {
                        id: "txn-1".into(),
                        amount: 20.0,
                        description: "Synthetic dividend".into(),
                        posted: None,
                        memo: Some("Retained memo".into()),
                    }],
                })
                .collect(),
            ..Default::default()
        }
    }

    fn save(
        conn: &mut Connection,
        provider: Provider,
        remotes: &[RemoteAccount],
        choices: Vec<AccountChoice>,
    ) -> AccountReview {
        let revision = discover(conn, provider, remotes).unwrap().revision;
        save_choices(
            conn,
            provider,
            remotes,
            &SaveAccountChoices { revision, choices },
        )
        .unwrap()
    }

    fn create_all(conn: &mut Connection, provider: Provider, remotes: &[RemoteAccount]) {
        save(
            conn,
            provider,
            remotes,
            remotes
                .iter()
                .map(|a| AccountChoice::Create {
                    remote_id: a.remote_id.clone(),
                })
                .collect(),
        );
    }

    fn sync_snap(
        conn: &mut Connection,
        data: &[(SnapAccount, Vec<SnapPosition>)],
    ) -> Result<snaptrade::SnapTradeSyncSummary, String> {
        let remotes = snap_remotes(data);
        let plan = plan_sync(conn, Provider::SnapTrade, &remotes)?;
        snaptrade::apply_sync(conn, &remotes, &plan, data, TODAY, NOW)
    }

    fn sync_simplefin(
        conn: &mut Connection,
        data: &SimpleFinAccountSet,
    ) -> Result<simplefin::SimpleFinSyncSummary, String> {
        let remotes = simplefin::remote_accounts(&data.accounts);
        let plan = plan_sync(conn, Provider::SimpleFin, &remotes)?;
        simplefin::apply_sync(conn, &remotes, &plan, data, TODAY, NOW)
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn local_id(conn: &Connection, provider: Provider, remote_id: &str) -> i64 {
        conn.query_row(
            "SELECT account_id FROM account_sync_selections WHERE provider = ?1 AND remote_id = ?2",
            params![provider.as_str(), remote_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn is_active(conn: &Connection, id: i64) -> bool {
        conn.query_row("SELECT is_active FROM accounts WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn local_account(
        conn: &Connection,
        name: &str,
        currency: &str,
        kind: &str,
        reference: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO accounts (name, institution, account_type, currency, jurisdiction, notes, \
             connector_kind, connector_ref, created_at) \
             VALUES (?1, 'Robinhood', 'brokerage', ?2, 'CA', 'My notes', ?3, ?4, '2020-01-01T00:00:00Z')",
            params![name, currency, kind, reference],
        ).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn discovery_is_read_only_masks_numbers_and_excludes_new_accounts_for_both_providers() {
        let mut conn = database();
        let snap = snap_accounts();
        let sf = simplefin_accounts();
        for (provider, remotes) in [
            (Provider::SnapTrade, snap_remotes(&snap)),
            (
                Provider::SimpleFin,
                simplefin::remote_accounts(&sf.accounts),
            ),
        ] {
            let before = conn.total_changes();
            let review = discover(&conn, provider, &remotes).unwrap();
            assert!(review
                .accounts
                .iter()
                .all(|a| a.selection == SelectionState::Unreviewed));
            let plan = plan_sync(&conn, provider, &remotes).unwrap();
            assert!(plan.targets.is_empty());
            assert_eq!(plan.accounts_needing_review, 2);
            assert!(plan
                .require_selected()
                .unwrap_err()
                .contains("Choose accounts"));
            assert_eq!(conn.total_changes(), before);
        }
        let review = discover(&conn, Provider::SnapTrade, &snap_remotes(&snap)).unwrap();
        assert_eq!(
            review.accounts[0].remote.masked_number.as_deref(),
            Some("****0580")
        );
        assert_eq!(
            review.accounts[1].remote.masked_number.as_deref(),
            Some("****7986")
        );
        let json = serde_json::to_string(&review).unwrap();
        assert!(!json.contains("12340580"));
        assert!(!json.contains("98767986"));
        assert!(sync_snap(&mut conn, &snap).is_err());
        assert!(sync_simplefin(&mut conn, &sf).is_err());
        for table in [
            "accounts",
            "balance_snapshots",
            "holdings",
            "transactions",
            "account_sync_selections",
        ] {
            assert_eq!(count(&conn, table), 0);
        }
    }

    #[test]
    fn explicit_create_is_idempotent_and_keeps_distinct_individual_accounts_separate() {
        let mut conn = database();
        let snap = snap_accounts();
        let remotes = snap_remotes(&snap);
        create_all(&mut conn, Provider::SnapTrade, &remotes);
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "balance_snapshots"), 0);
        let id1 = local_id(&conn, Provider::SnapTrade, "snap-1");
        let id2 = local_id(&conn, Provider::SnapTrade, "snap-2");
        assert_ne!(id1, id2);
        for _ in 0..3 {
            assert_eq!(sync_snap(&mut conn, &snap).unwrap().accounts_synced, 2);
            assert_eq!(local_id(&conn, Provider::SnapTrade, "snap-1"), id1);
        }
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "balance_snapshots"), 2);
        assert_eq!(count(&conn, "holdings"), 2);
        assert_eq!(active_count(&conn, Provider::SnapTrade).unwrap(), 2);
    }

    #[test]
    fn simplefin_create_and_repeat_sync_are_idempotent() {
        let mut conn = database();
        let data = simplefin_accounts();
        create_all(
            &mut conn,
            Provider::SimpleFin,
            &simplefin::remote_accounts(&data.accounts),
        );
        for _ in 0..3 {
            assert_eq!(sync_simplefin(&mut conn, &data).unwrap().accounts_synced, 2);
        }
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "transactions"), 2);
        assert_eq!(count(&conn, "balance_snapshots"), 2);
        assert_eq!(count(&conn, "holdings"), 2);
    }

    #[test]
    fn linking_preserves_history_metadata_and_excludes_the_old_provider_durably() {
        let mut conn = database();
        let mut sf = simplefin_accounts();
        let sf_remotes = simplefin::remote_accounts(&sf.accounts);
        create_all(&mut conn, Provider::SimpleFin, &sf_remotes);
        sync_simplefin(&mut conn, &sf).unwrap();
        let original = local_id(&conn, Provider::SimpleFin, "sf-1");
        let distinct = local_id(&conn, Provider::SimpleFin, "sf-2");
        conn.execute(
            "UPDATE accounts SET name = 'My Individual (0580)', notes = 'Keep my notes', \
             jurisdiction = 'CA', created_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            [original],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO balance_snapshots (account_id, snapshot_date, balance, currency) \
             VALUES (?1, '2020-01-01', 500, 'USD')",
            [original],
        )
        .unwrap();
        conn.execute(
            "UPDATE transactions SET category = 'Custom', flow_override = 'income' WHERE account_id = ?1",
            [original],
        ).unwrap();
        conn.execute(
            "INSERT INTO goals (name, target_amount, linked_account_ids) VALUES ('My goal', 9000, ?1)",
            [format!("[{original}]")],
        ).unwrap();
        let txn_id: i64 = conn
            .query_row(
                "SELECT id FROM transactions WHERE account_id = ?1",
                [original],
                |r| r.get(0),
            )
            .unwrap();
        let mut snap = snap_accounts();
        let snap_remotes = snap_remotes(&snap);
        save(
            &mut conn,
            Provider::SnapTrade,
            &snap_remotes,
            vec![AccountChoice::Link {
                remote_id: "snap-1".into(),
                account_id: original,
            }],
        );
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "balance_snapshots"), 3);
        assert_eq!(count(&conn, "holdings"), 2);
        assert_eq!(local_id(&conn, Provider::SnapTrade, "snap-1"), original);
        snap[0].0.balance_total = Some(2500.0);
        let summary = sync_snap(&mut conn, &snap).unwrap();
        assert_eq!(summary.accounts_synced, 1);
        assert_eq!(summary.accounts_needing_review, 1);
        sf.accounts[0].balance = Some(1.0);
        sf.accounts[0].transactions[0].amount = 9999.0;
        sf.accounts[1].balance = Some(1200.0);
        for _ in 0..3 {
            // Even a SimpleFIN server ignoring its account filter cannot write the old source.
            assert_eq!(sync_simplefin(&mut conn, &sf).unwrap().accounts_synced, 1);
        }
        apply_schema(&conn).unwrap();
        assert_eq!(count(&conn, "accounts"), 2);
        assert!(is_active(&conn, original));
        assert!(is_active(&conn, distinct));
        assert_eq!(active_count(&conn, Provider::SnapTrade).unwrap(), 1);
        assert_eq!(active_count(&conn, Provider::SimpleFin).unwrap(), 1);
        let metadata: (String, String, String, String, String) = conn.query_row(
            "SELECT name, notes, jurisdiction, created_at, connector_kind FROM accounts WHERE id = ?1",
            [original], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).unwrap();
        assert_eq!(
            metadata,
            (
                "My Individual (0580)".into(),
                "Keep my notes".into(),
                "CA".into(),
                "2020-01-01T00:00:00Z".into(),
                "snaptrade".into(),
            )
        );
        let transaction: (i64, f64, String, String, String) = conn.query_row(
            "SELECT id, amount, category, flow_override, memo FROM transactions WHERE account_id = ?1",
            [original], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).unwrap();
        assert_eq!(
            transaction,
            (
                txn_id,
                20.0,
                "Custom".into(),
                "income".into(),
                "Retained memo".into()
            )
        );
        let latest: f64 = conn.query_row(
            "SELECT balance FROM balance_snapshots WHERE account_id = ?1 ORDER BY snapshot_date DESC LIMIT 1",
            [original], |r| r.get(0),
        ).unwrap();
        assert_eq!(latest, 2500.0);
        let goal_ids: String = conn
            .query_row("SELECT linked_account_ids FROM goals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(goal_ids, format!("[{original}]"));
        save(
            &mut conn,
            Provider::SimpleFin,
            &sf_remotes,
            vec![AccountChoice::Ignore {
                remote_id: "sf-1".into(),
            }],
        );
        assert!(is_active(&conn, original));
        let tx = conn.transaction().unwrap();
        pause_provider(&tx, Provider::SimpleFin).unwrap();
        tx.commit().unwrap();
        assert!(is_active(&conn, original));
        assert!(!is_active(&conn, distinct));
        assert!(sync_simplefin(&mut conn, &sf).is_err());
    }

    #[test]
    fn manual_link_preserves_account_id_and_does_not_merge_by_similar_metadata() {
        let mut conn = database();
        let original = local_account(&conn, "Individual (0580)", "USD", "manual", None);
        let distinct = local_account(&conn, "Individual (7986)", "USD", "manual", None);
        let snap = snap_accounts();
        let remotes = snap_remotes(&snap);
        let review = discover(&conn, Provider::SnapTrade, &remotes).unwrap();
        assert_eq!(review.accounts[0].local_account_id, None);
        assert!(plan_sync(&conn, Provider::SnapTrade, &remotes)
            .unwrap()
            .targets
            .is_empty());
        save(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            vec![AccountChoice::Link {
                remote_id: "snap-1".into(),
                account_id: original,
            }],
        );
        sync_snap(&mut conn, &snap).unwrap();
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(local_id(&conn, Provider::SnapTrade, "snap-1"), original);
        let kind: String = conn
            .query_row(
                "SELECT connector_kind FROM accounts WHERE id = ?1",
                [distinct],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "manual");
    }

    #[test]
    fn ignored_imported_duplicate_stays_hidden_with_history_and_can_only_resume_its_own_row() {
        let mut conn = database();
        let original = local_account(&conn, "Original Individual (0580)", "USD", "manual", None);
        let snap = snap_accounts();
        let remotes = snap_remotes(&snap);
        create_all(&mut conn, Provider::SnapTrade, &remotes);
        sync_snap(&mut conn, &snap).unwrap();
        let duplicate = local_id(&conn, Provider::SnapTrade, "snap-1");
        let revision = discover(&conn, Provider::SnapTrade, &remotes)
            .unwrap()
            .revision;
        assert!(save_choices(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            &SaveAccountChoices {
                revision,
                choices: vec![AccountChoice::Link {
                    remote_id: "snap-1".into(),
                    account_id: original
                }],
            }
        )
        .unwrap_err()
        .contains("history"));
        save(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            vec![AccountChoice::Ignore {
                remote_id: "snap-1".into(),
            }],
        );
        for _ in 0..3 {
            assert_eq!(sync_snap(&mut conn, &snap).unwrap().accounts_synced, 1);
            apply_schema(&conn).unwrap();
        }
        assert!(!is_active(&conn, duplicate));
        assert!(is_active(&conn, original));
        assert_eq!(count(&conn, "balance_snapshots"), 2);
        assert_eq!(count(&conn, "holdings"), 2);
        let visible_snapshots: i64 = conn.query_row(
            "SELECT COUNT(*) FROM balance_snapshots b JOIN accounts a ON a.id = b.account_id WHERE a.is_active = 1",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(visible_snapshots, 1);
        save(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            vec![AccountChoice::Sync {
                remote_id: "snap-1".into(),
            }],
        );
        assert!(is_active(&conn, duplicate));
        assert_eq!(local_id(&conn, Provider::SnapTrade, "snap-1"), duplicate);
        assert_eq!(count(&conn, "accounts"), 3);
    }

    #[test]
    fn ignore_and_delete_persist_for_both_providers_without_reactivation() {
        for provider in [Provider::SnapTrade, Provider::SimpleFin] {
            let mut conn = database();
            let snap = snap_accounts();
            let sf = simplefin_accounts();
            let remotes = match provider {
                Provider::SnapTrade => snap_remotes(&snap),
                Provider::SimpleFin => simplefin::remote_accounts(&sf.accounts),
            };
            create_all(&mut conn, provider, &remotes);
            let first = local_id(&conn, provider, &remotes[0].remote_id);
            let second = local_id(&conn, provider, &remotes[1].remote_id);
            save(
                &mut conn,
                provider,
                &remotes,
                vec![AccountChoice::Ignore {
                    remote_id: remotes[0].remote_id.clone(),
                }],
            );
            deactivate_account(&mut conn, second).unwrap();
            apply_schema(&conn).unwrap();
            for _ in 0..3 {
                assert!(plan_sync(&conn, provider, &remotes)
                    .unwrap()
                    .targets
                    .is_empty());
                match provider {
                    Provider::SnapTrade => {
                        assert!(sync_snap(&mut conn, &snap).is_err());
                    }
                    Provider::SimpleFin => {
                        assert!(sync_simplefin(&mut conn, &sf).is_err());
                    }
                }
            }
            assert!(!is_active(&conn, first));
            assert!(!is_active(&conn, second));
            assert_eq!(active_count(&conn, provider).unwrap(), 0);
            assert_eq!(count(&conn, "balance_snapshots"), 0);
        }
    }

    #[test]
    fn migration_preserves_active_and_inactive_legacy_accounts_and_saved_choices() {
        let mut conn = database();
        let active = local_account(&conn, "Legacy active", "USD", "snaptrade", Some("snap-1"));
        let inactive = local_account(&conn, "Legacy deleted", "USD", "snaptrade", Some("snap-2"));
        conn.execute(
            "UPDATE accounts SET is_active = 0 WHERE id = ?1",
            [inactive],
        )
        .unwrap();
        apply_schema(&conn).unwrap();
        let remotes = snap_remotes(&snap_accounts());
        let review = discover(&conn, Provider::SnapTrade, &remotes).unwrap();
        assert_eq!(review.accounts[0].selection, SelectionState::Sync);
        assert_eq!(review.accounts[0].local_account_id, Some(active));
        assert_eq!(review.accounts[1].selection, SelectionState::Ignore);
        assert_eq!(review.accounts[1].local_account_id, Some(inactive));
        save(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            vec![AccountChoice::Ignore {
                remote_id: "snap-1".into(),
            }],
        );
        apply_schema(&conn).unwrap();
        assert!(plan_sync(&conn, Provider::SnapTrade, &remotes)
            .unwrap()
            .targets
            .is_empty());
        assert!(!is_active(&conn, active));
        assert!(!is_active(&conn, inactive));
    }

    #[test]
    fn invalid_or_duplicate_mapping_batches_are_rejected_atomically() {
        let mut conn = database();
        let target = local_account(&conn, "Target", "USD", "manual", None);
        let wrong_currency = local_account(&conn, "CAD account", "CAD", "manual", None);
        let wrong_type = local_account(&conn, "Retirement", "USD", "manual", None);
        conn.execute(
            "UPDATE accounts SET account_type = 'roth_ira' WHERE id = ?1",
            [wrong_type],
        )
        .unwrap();
        let unsupported = local_account(&conn, "Direct", "USD", "teller", Some("teller-1"));
        let hidden = local_account(&conn, "Hidden", "USD", "manual", None);
        deactivate_account(&mut conn, hidden).unwrap();
        let remotes = snap_remotes(&snap_accounts());
        let initial = discover(&conn, Provider::SnapTrade, &remotes)
            .unwrap()
            .revision;
        for id in [99999, wrong_currency, wrong_type, unsupported, hidden] {
            let result = save_choices(
                &mut conn,
                Provider::SnapTrade,
                &remotes,
                &SaveAccountChoices {
                    revision: initial.clone(),
                    choices: vec![
                        AccountChoice::Create {
                            remote_id: "snap-1".into(),
                        },
                        AccountChoice::Link {
                            remote_id: "snap-2".into(),
                            account_id: id,
                        },
                    ],
                },
            );
            assert!(result.is_err());
            assert_eq!(
                discover(&conn, Provider::SnapTrade, &remotes)
                    .unwrap()
                    .revision,
                initial
            );
        }
        for choices in [
            vec![
                AccountChoice::Link {
                    remote_id: "snap-1".into(),
                    account_id: target,
                },
                AccountChoice::Link {
                    remote_id: "snap-2".into(),
                    account_id: target,
                },
            ],
            vec![
                AccountChoice::Create {
                    remote_id: "snap-1".into(),
                },
                AccountChoice::Ignore {
                    remote_id: "snap-1".into(),
                },
            ],
            vec![AccountChoice::Create {
                remote_id: "stale-provider-id".into(),
            }],
            vec![AccountChoice::Sync {
                remote_id: "snap-1".into(),
            }],
        ] {
            assert!(save_choices(
                &mut conn,
                Provider::SnapTrade,
                &remotes,
                &SaveAccountChoices {
                    revision: initial.clone(),
                    choices,
                }
            )
            .is_err());
            assert_eq!(
                discover(&conn, Provider::SnapTrade, &remotes)
                    .unwrap()
                    .revision,
                initial
            );
        }
        assert_eq!(count(&conn, "accounts"), 5);
        assert_eq!(count(&conn, "account_sync_selections"), 0);
    }

    #[test]
    fn database_write_failure_rolls_back_the_entire_batch() {
        let mut conn = database();
        let remotes = snap_remotes(&snap_accounts());
        conn.execute_batch(
            "CREATE TRIGGER fail_second_create BEFORE INSERT ON accounts \
             WHEN NEW.connector_ref = 'snap-2' BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
        ).unwrap();
        let revision = discover(&conn, Provider::SnapTrade, &remotes)
            .unwrap()
            .revision;
        let error = save_choices(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            &SaveAccountChoices {
                revision,
                choices: remotes
                    .iter()
                    .map(|a| AccountChoice::Create {
                        remote_id: a.remote_id.clone(),
                    })
                    .collect(),
            },
        )
        .unwrap_err();
        assert!(error.contains("synthetic failure"));
        assert_eq!(count(&conn, "accounts"), 0);
        assert_eq!(count(&conn, "account_sync_selections"), 0);
    }

    #[test]
    fn stale_save_and_inflight_sync_cannot_overwrite_new_choices_for_either_provider() {
        for provider in [Provider::SnapTrade, Provider::SimpleFin] {
            let mut conn = database();
            let snap = snap_accounts();
            let sf = simplefin_accounts();
            let remotes = match provider {
                Provider::SnapTrade => snap_remotes(&snap),
                Provider::SimpleFin => simplefin::remote_accounts(&sf.accounts),
            };
            create_all(&mut conn, provider, &remotes);
            let plan = plan_sync(&conn, provider, &remotes).unwrap();
            let stale_revision = plan.revision.clone();
            // This save takes place after selection was read, while a sync would be awaiting HTTP.
            save(
                &mut conn,
                provider,
                &remotes,
                vec![AccountChoice::Ignore {
                    remote_id: remotes[0].remote_id.clone(),
                }],
            );
            assert!(save_choices(
                &mut conn,
                provider,
                &remotes,
                &SaveAccountChoices {
                    revision: stale_revision,
                    choices: vec![AccountChoice::Sync {
                        remote_id: remotes[0].remote_id.clone()
                    }],
                }
            )
            .unwrap_err()
            .contains("changed"));
            let result = match provider {
                Provider::SnapTrade => {
                    snaptrade::apply_sync(&mut conn, &remotes, &plan, &snap, TODAY, NOW).map(|_| ())
                }
                Provider::SimpleFin => {
                    simplefin::apply_sync(&mut conn, &remotes, &plan, &sf, TODAY, NOW).map(|_| ())
                }
            };
            assert!(result.unwrap_err().contains("changed while syncing"));
            assert_eq!(count(&conn, "balance_snapshots"), 0);
            assert_eq!(count(&conn, "holdings"), 0);
            assert_eq!(count(&conn, "transactions"), 0);
            assert!(!is_active(
                &conn,
                local_id(&conn, provider, &remotes[0].remote_id)
            ));
        }
    }

    #[test]
    fn missing_selected_simplefin_account_rolls_back_without_recording_success() {
        let mut conn = database();
        let mut sf = simplefin_accounts();
        let remotes = simplefin::remote_accounts(&sf.accounts);
        create_all(&mut conn, Provider::SimpleFin, &remotes);
        let plan = plan_sync(&conn, Provider::SimpleFin, &remotes).unwrap();
        sf.accounts.pop();
        sf.errors
            .push("The second institution needs authentication.".into());
        let error = simplefin::apply_sync(&mut conn, &remotes, &plan, &sf, TODAY, NOW).unwrap_err();
        assert!(error.contains("authentication"));
        assert_eq!(count(&conn, "balance_snapshots"), 0);
        assert_eq!(count(&conn, "transactions"), 0);
        let marker: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'simplefin_last_synced_at'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(marker, None);
    }

    #[test]
    fn direct_questrade_reconciliation_remains_authoritative_and_never_reactivates_ignored_rows() {
        let mut conn = database();
        let mut snap = snap_accounts();
        snap[0].0.institution_name = Some("Questrade".into());
        let remotes = snap_remotes(&snap);
        create_all(&mut conn, Provider::SnapTrade, &remotes);
        let aggregated = local_id(&conn, Provider::SnapTrade, "snap-1");
        let direct = local_account(&conn, "Direct Questrade", "USD", "questrade", Some("qt-1"));
        reconcile_aggregated_questrade_accounts(&conn).unwrap();
        assert!(!is_active(&conn, aggregated));
        let plan = plan_sync(&conn, Provider::SnapTrade, &remotes).unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].remote_id, "snap-2");
        assert_eq!(sync_snap(&mut conn, &snap).unwrap().accounts_synced, 1);
        let review = discover(&conn, Provider::SnapTrade, &remotes).unwrap();
        assert!(review.accounts[0]
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("Direct Questrade"));
        assert!(save_choices(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            &SaveAccountChoices {
                revision: review.revision,
                choices: vec![AccountChoice::Sync {
                    remote_id: "snap-1".into()
                }],
            }
        )
        .is_err());
        deactivate_account(&mut conn, direct).unwrap();
        assert_eq!(sync_snap(&mut conn, &snap).unwrap().accounts_synced, 1);
        assert!(!is_active(&conn, aggregated));
    }

    #[test]
    fn returning_to_a_previous_provider_requires_the_same_remote_id_and_explicit_resume() {
        let mut conn = database();
        let sf = simplefin_accounts();
        let mut sf_remotes = simplefin::remote_accounts(&sf.accounts);
        create_all(&mut conn, Provider::SimpleFin, &sf_remotes);
        sync_simplefin(&mut conn, &sf).unwrap();
        let original = local_id(&conn, Provider::SimpleFin, "sf-1");
        let snap = snap_accounts();
        let snap_remotes = snap_remotes(&snap);
        save(
            &mut conn,
            Provider::SnapTrade,
            &snap_remotes,
            vec![AccountChoice::Link {
                remote_id: "snap-1".into(),
                account_id: original,
            }],
        );
        let mut different_id = sf_remotes[0].clone();
        different_id.remote_id = "sf-replacement-id".into();
        sf_remotes.push(different_id);
        let review = discover(&conn, Provider::SimpleFin, &sf_remotes).unwrap();
        assert!(save_choices(
            &mut conn,
            Provider::SimpleFin,
            &sf_remotes,
            &SaveAccountChoices {
                revision: review.revision,
                choices: vec![AccountChoice::Link {
                    remote_id: "sf-replacement-id".into(),
                    account_id: original
                }],
            }
        )
        .unwrap_err()
        .contains("different account ID"));
        save(
            &mut conn,
            Provider::SimpleFin,
            &sf_remotes,
            vec![AccountChoice::Sync {
                remote_id: "sf-1".into(),
            }],
        );
        assert!(plan_sync(&conn, Provider::SnapTrade, &snap_remotes)
            .unwrap()
            .targets
            .is_empty());
        assert_eq!(local_id(&conn, Provider::SimpleFin, "sf-1"), original);
        assert_eq!(sync_simplefin(&mut conn, &sf).unwrap().accounts_synced, 2);
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "transactions"), 2);
    }

    #[test]
    fn disconnect_then_resume_keeps_exclusions_and_reuses_existing_rows_for_both_providers() {
        for provider in [Provider::SnapTrade, Provider::SimpleFin] {
            let mut conn = database();
            let remotes = match provider {
                Provider::SnapTrade => snap_remotes(&snap_accounts()),
                Provider::SimpleFin => simplefin::remote_accounts(&simplefin_accounts().accounts),
            };
            create_all(&mut conn, provider, &remotes);
            let original = local_id(&conn, provider, &remotes[0].remote_id);
            let tx = conn.transaction().unwrap();
            pause_provider(&tx, provider).unwrap();
            tx.commit().unwrap();
            apply_schema(&conn).unwrap();
            assert!(plan_sync(&conn, provider, &remotes)
                .unwrap()
                .targets
                .is_empty());
            assert_eq!(active_count(&conn, provider).unwrap(), 0);
            save(
                &mut conn,
                provider,
                &remotes,
                vec![AccountChoice::Sync {
                    remote_id: remotes[0].remote_id.clone(),
                }],
            );
            let plan = plan_sync(&conn, provider, &remotes).unwrap();
            assert_eq!(plan.targets.len(), 1);
            assert_eq!(plan.targets[0].account_id, original);
            assert_eq!(count(&conn, "accounts"), 2);
        }
    }

    #[test]
    fn provider_id_ambiguity_and_unknown_currency_cannot_create_accounts() {
        let mut conn = database();
        let mut remotes = snap_remotes(&snap_accounts());
        remotes[1].remote_id = remotes[0].remote_id.clone();
        assert!(discover(&conn, Provider::SnapTrade, &remotes).is_err());
        remotes[1].remote_id.clear();
        assert!(discover(&conn, Provider::SnapTrade, &remotes).is_err());
        remotes = snap_remotes(&snap_accounts());
        remotes[0].currency = None;
        let review = discover(&conn, Provider::SnapTrade, &remotes).unwrap();
        assert!(review.accounts[0].linkable_account_ids.is_empty());
        assert!(save_choices(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            &SaveAccountChoices {
                revision: review.revision,
                choices: vec![AccountChoice::Create {
                    remote_id: "snap-1".into()
                }],
            }
        )
        .unwrap_err()
        .contains("currency"));
        assert_eq!(count(&conn, "accounts"), 0);
        local_account(&conn, "Legacy 1", "USD", "snaptrade", Some("snap-1"));
        local_account(&conn, "Legacy 2", "USD", "snaptrade", Some("snap-1"));
        apply_schema(&conn).unwrap();
        assert!(discover(&conn, Provider::SnapTrade, &remotes)
            .unwrap_err()
            .contains("unambiguous"));
        assert_eq!(count(&conn, "accounts"), 2);
        assert_eq!(count(&conn, "account_sync_selections"), 0);
    }

    #[test]
    fn provider_metadata_changes_make_a_saved_review_stale() {
        let mut conn = database();
        let mut remotes = snap_remotes(&snap_accounts());
        let review = discover(&conn, Provider::SnapTrade, &remotes).unwrap();
        remotes[0].currency = Some("CAD".into());
        assert!(save_choices(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            &SaveAccountChoices {
                revision: review.revision,
                choices: vec![AccountChoice::Create {
                    remote_id: "snap-1".into()
                }],
            }
        )
        .unwrap_err()
        .contains("changed"));
        assert_eq!(count(&conn, "accounts"), 0);
    }

    #[test]
    fn inconsistent_simplefin_currency_or_duplicate_payload_ids_roll_back() {
        let mut conn = database();
        let original = simplefin_accounts();
        let remotes = simplefin::remote_accounts(&original.accounts);
        create_all(&mut conn, Provider::SimpleFin, &remotes);
        let plan = plan_sync(&conn, Provider::SimpleFin, &remotes).unwrap();
        let mut changed_currency = original.clone();
        changed_currency.accounts[1].currency = "CAD".into();
        let mut duplicate_id = original.clone();
        duplicate_id.accounts.push(original.accounts[0].clone());
        for data in [changed_currency, duplicate_id] {
            assert!(simplefin::apply_sync(&mut conn, &remotes, &plan, &data, TODAY, NOW).is_err());
            assert_eq!(count(&conn, "balance_snapshots"), 0);
            assert_eq!(count(&conn, "transactions"), 0);
            assert_eq!(count(&conn, "holdings"), 0);
        }
    }

    #[test]
    fn linked_questrade_with_a_user_institution_label_still_obeys_direct_reconciliation() {
        let mut conn = database();
        let original = local_account(&conn, "My retirement", "USD", "manual", None);
        conn.execute(
            "UPDATE accounts SET institution = 'My broker alias' WHERE id = ?1",
            [original],
        )
        .unwrap();
        let mut snap = snap_accounts();
        snap[0].0.institution_name = Some("Questrade".into());
        let remotes = snap_remotes(&snap);
        save(
            &mut conn,
            Provider::SnapTrade,
            &remotes,
            vec![
                AccountChoice::Link {
                    remote_id: "snap-1".into(),
                    account_id: original,
                },
                AccountChoice::Create {
                    remote_id: "snap-2".into(),
                },
            ],
        );
        local_account(&conn, "Direct Questrade", "USD", "questrade", Some("qt-1"));
        reconcile_aggregated_questrade_accounts(&conn).unwrap();
        assert!(!is_active(&conn, original));
        assert_eq!(active_count(&conn, Provider::SnapTrade).unwrap(), 1);
        assert_eq!(sync_snap(&mut conn, &snap).unwrap().accounts_synced, 1);
        let institution: String = conn
            .query_row(
                "SELECT institution FROM accounts WHERE id = ?1",
                [original],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(institution, "My broker alias");
    }
}
