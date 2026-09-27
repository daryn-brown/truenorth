use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::connector::simplefin::{SimpleFinAccount, SimpleFinAccountSet, SimpleFinIssue};

const KEY: &str = "simplefin_connection_health";

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct HealthState {
    pub last_attempt_at: Option<String>,
    pub last_response_at: Option<String>,
    pub last_error: Option<String>,
    pub app_auth_required: bool,
    requests: Vec<i64>,
    issues: Vec<SimpleFinIssue>,
    connections: Vec<StoredConnection>,
}

#[derive(Serialize, Deserialize)]
struct StoredConnection {
    id: String,
    name: String,
    last_seen_at: String,
    accounts: Vec<StoredAccount>,
}

#[derive(Serialize, Deserialize)]
struct StoredAccount {
    id: i64,
    external_id: String,
    balance_at: Option<i64>,
    last_seen_at: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceStatus {
    Current,
    Unknown,
    Stale,
    Missing,
    Error,
    ReauthRequired,
}

#[derive(Debug, Serialize)]
pub struct BalanceHealth {
    pub status: BalanceStatus,
    pub balance_as_of: Option<String>,
    pub message: Option<String>,
}

impl BalanceHealth {
    fn flag(&mut self, status: BalanceStatus, message: impl Into<String>) {
        if status > self.status {
            self.status = status;
            self.message = Some(message.into());
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AccountHealth {
    pub account_id: i64,
    pub name: String,
    pub health: BalanceHealth,
}

#[derive(Debug, Serialize)]
pub struct ConnectionHealth {
    pub id: String,
    pub name: String,
    pub status: BalanceStatus,
    pub messages: Vec<String>,
    pub accounts: Vec<AccountHealth>,
}

pub(super) fn stamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(super) fn load(conn: &Connection) -> rusqlite::Result<HealthState> {
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [KEY],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|value| {
        serde_json::from_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    })
    .transpose()
    .map(|state| state.unwrap_or_default())
}

pub(super) fn save(conn: &Connection, state: &HealthState) -> rusqlite::Result<()> {
    let json = serde_json::to_string(state)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        params![KEY, json],
    )?;
    Ok(())
}

pub(super) fn source_time(account: &SimpleFinAccount, now: DateTime<Utc>) -> Option<i64> {
    account.balance.filter(|balance| balance.is_finite())?;
    account
        .balance_date
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .filter(|at| *at <= now + Duration::minutes(5))
        .filter(|at| at.date_naive() <= now.date_naive())
        .map(|at| at.timestamp())
}

pub(super) fn connection_key(account: &SimpleFinAccount) -> String {
    account.connection_id.clone().unwrap_or_else(|| {
        format!(
            "legacy:{}",
            account.institution.as_deref().unwrap_or("SimpleFIN")
        )
    })
}

pub(super) fn account_data_failed(data: &SimpleFinAccountSet, account: &SimpleFinAccount) -> bool {
    data.issues.iter().any(|issue| {
        let failure = issue.code == "gen.auth"
            || (issue.code == "con.auth" && issue.conn_id.is_some())
            || (issue.code == "act.failed" && issue.account_id.is_some());
        failure
            && issue
                .conn_id
                .as_deref()
                .is_none_or(|id| account.connection_id.as_deref() == Some(id))
            && issue
                .account_id
                .as_deref()
                .is_none_or(|id| id == account.id)
    })
}

impl HealthState {
    pub fn reserve(&mut self, now: DateTime<Utc>, automatic: bool) -> Result<bool, String> {
        if automatic && self.app_auth_required {
            return Ok(false);
        }
        if let Some(last) = &self.last_attempt_at {
            let last = DateTime::parse_from_rfc3339(last).map_err(|error| error.to_string())?;
            let elapsed = now.signed_duration_since(last);
            if automatic && elapsed < Duration::hours(6) {
                return Ok(false);
            }
            if elapsed < Duration::minutes(1) {
                return Err("Wait at least one minute between SimpleFIN checks. Bank updates can take longer.".into());
            }
        }
        self.requests.retain(|at| *at > now.timestamp() - 86_400);
        if self.requests.len() >= 24 {
            return Err("This app has made 24 SimpleFIN requests in the last 24 hours. Wait before syncing again.".into());
        }
        self.requests.push(now.timestamp());
        self.last_attempt_at = Some(stamp(now));
        Ok(true)
    }

    pub fn token_saved(&mut self) {
        self.app_auth_required = false;
        self.issues.retain(|issue| issue.code != "gen.auth");
        self.last_error =
            Some("App token saved. Sync accounts to verify access and balances.".into());
    }

    pub fn balance_at(&self, id: i64) -> Option<i64> {
        self.connections
            .iter()
            .flat_map(|bank| &bank.accounts)
            .find(|account| account.id == id)
            .and_then(|account| account.balance_at)
    }

    pub fn response(&mut self, data: &SimpleFinAccountSet, now: DateTime<Utc>) {
        self.last_response_at = Some(stamp(now));
        self.last_error = None;
        self.app_auth_required = data.issues.iter().any(|issue| issue.code == "gen.auth");
        self.issues = data.issues.clone();
        for bank in &data.connections {
            self.ensure_connection(&bank.id, &bank.name, now);
        }
    }

    fn ensure_connection(&mut self, id: &str, name: &str, now: DateTime<Utc>) -> usize {
        if let Some(index) = self.connections.iter().position(|bank| bank.id == id) {
            self.connections[index].name = name.to_owned();
            self.connections[index].last_seen_at = stamp(now);
            index
        } else {
            self.connections.push(StoredConnection {
                id: id.to_owned(),
                name: name.to_owned(),
                last_seen_at: stamp(now),
                accounts: Vec::new(),
            });
            self.connections.len() - 1
        }
    }

    pub fn record_account(&mut self, id: i64, account: &SimpleFinAccount, now: DateTime<Utc>) {
        let previous = self.balance_at(id);
        let incoming = source_time(account, now);
        let (balance_at, error) = match incoming {
            Some(at) if previous.is_none_or(|before| at >= before) => (Some(at), None),
            Some(_) => (previous, Some("SimpleFIN returned an older balance. The newer saved balance was kept.".into())),
            None => (previous, Some("SimpleFIN did not return a valid dated balance. The last known balance was kept.".into())),
        };
        for bank in &mut self.connections {
            bank.accounts.retain(|stored| stored.id != id);
        }
        let index = self.ensure_connection(
            &connection_key(account),
            account.institution.as_deref().unwrap_or("SimpleFIN"),
            now,
        );
        self.connections[index].accounts.push(StoredAccount {
            id,
            external_id: account.id.clone(),
            balance_at,
            last_seen_at: stamp(now),
            error,
        });
    }

    pub fn messages(&self) -> Vec<String> {
        let mut messages: Vec<String> = self.last_error.iter().cloned().collect();
        for issue in &self.issues {
            let represented = self.connections.iter().any(|bank| {
                issue.conn_id.as_deref() == Some(&bank.id)
                    || (issue.conn_id.is_none()
                        && issue.account_id.as_deref().is_some_and(|id| {
                            bank.accounts
                                .iter()
                                .any(|account| account.external_id == id)
                        }))
            });
            if !represented && !messages.contains(&issue.msg) {
                messages.push(issue.msg.clone());
            }
        }
        messages
    }

    pub fn finish_response(&mut self) {
        self.connections.retain(|bank| {
            !bank.accounts.is_empty()
                || self.last_response_at.as_deref() == Some(bank.last_seen_at.as_str())
        });
    }

    fn health(
        &self,
        bank: &str,
        account: Option<&StoredAccount>,
        connected: bool,
    ) -> BalanceHealth {
        let mut health = BalanceHealth {
            status: BalanceStatus::Current,
            balance_as_of: None,
            message: None,
        };
        if !connected {
            health.flag(
                BalanceStatus::Unknown,
                "SimpleFIN is disconnected. These are saved balances.",
            );
            return health;
        }
        if let Some(error) = &self.last_error {
            health.flag(BalanceStatus::Unknown, error);
        }
        for issue in &self.issues {
            if issue.conn_id.as_deref().is_none_or(|id| id == bank)
                && issue
                    .account_id
                    .as_deref()
                    .is_none_or(|id| account.is_some_and(|account| account.external_id == id))
            {
                let status = if issue.code == "gen.auth"
                    || (issue.code == "con.auth" && issue.conn_id.is_some())
                {
                    BalanceStatus::ReauthRequired
                } else {
                    BalanceStatus::Error
                };
                health.flag(status, &issue.msg);
            }
        }
        if self.app_auth_required {
            health.status = BalanceStatus::ReauthRequired;
            health.message = Some("Replace this app's SimpleFIN token. This is not a request to reauthenticate every bank.".into());
        }
        health
    }

    pub fn connections(
        &self,
        conn: &Connection,
        connected: bool,
        now: DateTime<Utc>,
    ) -> rusqlite::Result<Vec<ConnectionHealth>> {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.name, a.institution, bs.snapshot_date, bs.source \
             FROM accounts a LEFT JOIN balance_snapshots bs ON bs.id = \
             (SELECT id FROM balance_snapshots WHERE account_id = a.id ORDER BY snapshot_date DESC LIMIT 1) \
             WHERE a.connector_kind = 'simplefin' AND a.is_active = 1 ORDER BY a.institution, a.name",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut result: Vec<ConnectionHealth> = Vec::new();
        for bank in &self.connections {
            let mut base = self.health(&bank.id, None, connected);
            if self
                .last_response_at
                .as_deref()
                .is_some_and(|at| at != bank.last_seen_at)
            {
                base.flag(
                    BalanceStatus::Missing,
                    "This bank was not returned by the last check. Saved balances are retained.",
                );
            }
            let messages = self
                .issues
                .iter()
                .filter(|issue| {
                    issue.conn_id.as_deref() == Some(&bank.id)
                        || (issue.conn_id.is_none()
                            && issue.account_id.as_deref().is_some_and(|id| {
                                bank.accounts
                                    .iter()
                                    .any(|account| account.external_id == id)
                            }))
                })
                .map(|issue| issue.msg.clone())
                .collect();
            result.push(ConnectionHealth {
                id: bank.id.clone(),
                name: bank.name.clone(),
                status: base.status,
                messages,
                accounts: Vec::new(),
            });
        }
        for (id, name, institution, snapshot_date, source) in rows {
            let stored = self.connections.iter().find_map(|bank| {
                bank.accounts
                    .iter()
                    .find(|account| account.id == id)
                    .map(|account| (bank, account))
            });
            let (bank_id, mut health) = if let Some((bank, account)) = stored {
                let mut health = self.health(&bank.id, Some(account), connected);
                health.balance_as_of = account
                    .balance_at
                    .and_then(|at| DateTime::from_timestamp(at, 0))
                    .map(stamp);
                if self
                    .last_response_at
                    .as_deref()
                    .is_some_and(|at| at != account.last_seen_at)
                {
                    health.flag(
                        BalanceStatus::Missing,
                        "This account was not returned. Its last known balance is still included.",
                    );
                }
                if let Some(error) = &account.error {
                    health.flag(BalanceStatus::Error, error);
                }
                match account.balance_at {
                    Some(at) if now.timestamp() - at >= 48 * 3600 => health.flag(
                        BalanceStatus::Stale, "This balance is at least 48 hours old. An app sync does not mean the bank refreshed it.",
                    ),
                    None => health.flag(BalanceStatus::Unknown, "Sync to verify the bank's balance date."),
                    _ => {}
                }
                (bank.id.clone(), health)
            } else {
                (
                    format!("unverified:{institution}"),
                    BalanceHealth {
                        status: BalanceStatus::Unknown,
                        balance_as_of: None,
                        message: Some(
                            "Sync once to check this bank's connection and source balance date."
                                .into(),
                        ),
                    },
                )
            };
            if source.as_deref() != Some("simplefin") {
                health.balance_as_of = snapshot_date.clone();
                if snapshot_date.is_some() && health.status <= BalanceStatus::Stale {
                    health.status = BalanceStatus::Unknown;
                    health.message = Some(
                        "This is a manually entered balance, not a verified bank update.".into(),
                    );
                }
            }
            if source.as_deref() == Some("simplefin") {
                if let (Some(as_of), Some(snapshot)) = (&health.balance_as_of, &snapshot_date) {
                    if !as_of.starts_with(snapshot) {
                        health.balance_as_of = snapshot_date.clone();
                        if health.status <= BalanceStatus::Stale {
                            health.status = BalanceStatus::Unknown;
                            health.message = Some("The returned source date differs from the latest saved balance. The last-known balance is retained.".into());
                        }
                    }
                }
            }
            if snapshot_date.is_none() {
                health.flag(
                    BalanceStatus::Missing,
                    "No usable balance is available. This account is not included in totals.",
                );
            } else if health.balance_as_of.is_none() {
                health.balance_as_of = snapshot_date;
            }
            let index = if let Some(index) = result.iter().position(|bank| bank.id == bank_id) {
                index
            } else {
                result.push(ConnectionHealth {
                    id: bank_id,
                    name: institution,
                    status: BalanceStatus::Current,
                    messages: Vec::new(),
                    accounts: Vec::new(),
                });
                result.len() - 1
            };
            result[index].status = result[index].status.max(health.status);
            result[index].accounts.push(AccountHealth {
                account_id: id,
                name,
                health,
            });
        }
        result.retain(|bank| {
            !bank.accounts.is_empty()
                || (connected
                    && self
                        .connections
                        .iter()
                        .any(|stored| stored.id == bank.id && stored.accounts.is_empty()))
        });
        for bank in &mut result {
            if bank.accounts.is_empty() {
                bank.status = bank.status.max(BalanceStatus::Missing);
                bank.messages
                    .push("No active account balances are available for this connection.".into());
            }
        }
        result.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-27T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn rate_budget_is_rolling_and_survives_a_reload() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let mut state = HealthState::default();
        assert!(state.reserve(now(), false).unwrap());
        assert!(!state.reserve(now() + Duration::hours(5), true).unwrap());
        assert!(state.reserve(now() + Duration::seconds(59), false).is_err());
        for minute in 1..24 {
            state
                .reserve(now() + Duration::minutes(minute), false)
                .unwrap();
        }
        save(&conn, &state).unwrap();
        let mut reloaded = load(&conn).unwrap();
        assert!(reloaded.reserve(now() + Duration::hours(6), false).is_err());
        assert!(reloaded.reserve(now() + Duration::days(1), false).unwrap());
    }

    #[test]
    fn auth_errors_stay_scoped_and_unknown_codes_remain_visible() {
        let mut state = HealthState::default();
        state.issues.push(SimpleFinIssue {
            code: "con.auth".into(),
            msg: "Approve bank A".into(),
            conn_id: Some("a".into()),
            account_id: None,
        });
        assert_eq!(
            state.health("a", None, true).status,
            BalanceStatus::ReauthRequired
        );
        assert_eq!(state.health("b", None, true).status, BalanceStatus::Current);
        state.issues[0].code = "con.future".into();
        assert_eq!(state.health("a", None, true).status, BalanceStatus::Error);
        assert_eq!(state.messages(), vec!["Approve bank A"]);
    }

    #[test]
    fn app_access_is_separate_and_new_tokens_need_verification() {
        let mut state = HealthState::default();
        state.app_auth_required = true;
        assert!(!state.reserve(now(), true).unwrap());
        assert!(state
            .health("bank", None, true)
            .message
            .unwrap()
            .contains("app's SimpleFIN token"));
        state.token_saved();
        assert!(!state.app_auth_required);
        assert_eq!(
            state.health("bank", None, true).status,
            BalanceStatus::Unknown
        );
    }
}
