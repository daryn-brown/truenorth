//! Minimal SimpleFIN Bridge client.
//!
//! SimpleFIN is intentionally simple — there is no request signing or OAuth. The flow is:
//!
//! 1. The user creates a **setup token** at their SimpleFIN server (e.g. the SimpleFIN Bridge)
//!    and pastes it into TrueNorth. The token is a Base64-encoded **claim URL**.
//! 2. We POST to the claim URL once to receive a persistent **access URL** that embeds HTTP
//!    Basic credentials (`https://user:pass@host/simplefin`). The access URL is the only secret
//!    and is stored in the OS keychain.
//! 3. We GET `{access-url}/accounts` (Basic auth) to read balances + holdings.
//!
//! Responses are parsed defensively from `serde_json::Value`: the SimpleFIN Bridge embeds an
//! `org` object and a `holdings` array per account (extensions over the core protocol), while the
//! draft v2 protocol uses a separate `connections` list — we support both shapes.

use std::collections::HashMap;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const TRANSACTION_CALENDAR_DAYS: i64 = 90;

fn transaction_date_range(now: chrono::DateTime<chrono::Utc>) -> (String, String) {
    // Some upstream providers count both boundary dates. An elapsed 90-day interval can therefore
    // span 91 calendar dates, so request 89 elapsed days and pin the end to the same instant.
    let start = now - chrono::Duration::days(TRANSACTION_CALENDAR_DAYS - 1);
    (start.timestamp().to_string(), now.timestamp().to_string())
}

#[derive(Debug, Error)]
pub enum SimpleFinError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL error: {0}")]
    Url(String),

    #[error("SimpleFIN claim failed ({status}): {message}")]
    Claim { status: u16, message: String },

    #[error("SimpleFIN API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Unexpected SimpleFIN response: {0}")]
    Parse(String),
}

impl SimpleFinError {
    /// True when SimpleFIN rejected our credentials (HTTP 401/403) — used to surface a clear
    /// "reconnect" message and to advise that a compromised token should be disabled.
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            SimpleFinError::Api { status, .. } | SimpleFinError::Claim { status, .. }
                if *status == 401 || *status == 403
        )
    }
}

/// One holding (investment position) within a SimpleFIN account.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinHolding {
    pub symbol: String,
    pub shares: f64,
    pub market_value: Option<f64>,
    pub cost_basis: Option<f64>,
    pub currency: Option<String>,
}

/// One transaction within a SimpleFIN account. SimpleFIN reports `amount` as a signed string:
/// negative is money out (spending), positive is money in (income/refunds).
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinTransaction {
    /// SimpleFIN transaction id — stored as `connector_ref` for dedup across syncs.
    pub id: String,
    /// Signed amount in the account's currency (negative = outflow).
    pub amount: f64,
    pub description: String,
    /// `posted` (or `transacted_at`) as a UNIX epoch timestamp (seconds).
    pub posted: Option<i64>,
    pub memo: Option<String>,
}

/// One account as reported by SimpleFIN.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleFinAccount {
    /// SimpleFIN account id — stored as `connector_ref`.
    pub id: String,
    pub name: String,
    pub currency: String,
    /// Current balance in `currency`. The figure that flows into net worth.
    pub balance: Option<f64>,
    /// `balance-date` as a UNIX epoch timestamp (seconds).
    pub balance_date: Option<i64>,
    /// Institution / connection name, best-effort.
    pub institution: Option<String>,
    pub connection_id: Option<String>,
    pub holdings_reported: bool,
    pub holdings: Vec<SimpleFinHolding>,
    pub transactions: Vec<SimpleFinTransaction>,
}

/// The parsed `/accounts` response: the accounts plus any user-facing errors SimpleFIN returned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimpleFinAccountSet {
    pub accounts: Vec<SimpleFinAccount>,
    pub errors: Vec<String>,
    pub issues: Vec<SimpleFinIssue>,
    pub connections: Vec<SimpleFinConnection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimpleFinIssue {
    pub code: String,
    pub msg: String,
    pub conn_id: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimpleFinConnection {
    pub id: String,
    pub name: String,
}

/// Exchange a setup token for a persistent access URL.
///
/// The setup token is Base64 of a claim URL; we POST to it once and the response body is the
/// access URL (with embedded Basic credentials). A 403 means the token was already claimed or is
/// invalid — the caller should tell the user to disable it.
pub async fn claim_access_url(setup_token: &str) -> Result<String, SimpleFinError> {
    let claim_url = decode_setup_token(setup_token)?;
    let resp = http_client()?
        .post(&claim_url)
        .header("Content-Length", "0")
        .send()
        .await
        .map_err(|e| SimpleFinError::Http(e.without_url()))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| SimpleFinError::Http(e.without_url()))?;
    if !status.is_success() {
        return Err(SimpleFinError::Claim {
            status: status.as_u16(),
            message: extract_error_message(&text),
        });
    }
    let access_url = text.trim().to_string();
    let parsed = secure_url(&access_url)?;
    if parsed.username().is_empty() || parsed.password().is_none_or(str::is_empty) {
        return Err(SimpleFinError::Parse(
            "claim did not return complete app credentials".into(),
        ));
    }
    Ok(access_url)
}

/// Base64-decode a setup token into its claim URL, accepting standard or URL-safe alphabets.
fn decode_setup_token(token: &str) -> Result<String, SimpleFinError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(SimpleFinError::Parse("setup token is empty".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(token))
        .map_err(|e| SimpleFinError::Parse(format!("invalid setup token: {e}")))?;
    let url = String::from_utf8(bytes)
        .map_err(|e| SimpleFinError::Parse(format!("invalid setup token: {e}")))?
        .trim()
        .to_string();
    secure_url(&url)?;
    Ok(url)
}

fn secure_url(value: &str) -> Result<reqwest::Url, SimpleFinError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| SimpleFinError::Url("Invalid SimpleFIN URL".into()))?;
    if url.scheme() != "https" {
        return Err(SimpleFinError::Url(
            "SimpleFIN requires an HTTPS connection".into(),
        ));
    }
    Ok(url)
}

fn http_client() -> Result<reqwest::Client, SimpleFinError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(SimpleFinError::Http)
}

pub struct SimpleFinClient {
    access_url: String,
    http: reqwest::Client,
}

impl SimpleFinClient {
    pub fn new(access_url: impl Into<String>) -> Result<Self, SimpleFinError> {
        Ok(Self {
            access_url: access_url.into(),
            http: http_client()?,
        })
    }

    /// Fetch accounts with balances, holdings, and recent transactions. Explicit `start-date` and
    /// `end-date` values keep the request within SimpleFIN Bridge's 90-calendar-day limit; balances
    /// and holdings are always current regardless of the transaction window.
    pub async fn fetch_accounts(&self) -> Result<SimpleFinAccountSet, SimpleFinError> {
        let endpoint = accounts_endpoint(&self.access_url)?;
        // SimpleFIN expects both date bounds as UNIX epoch seconds.
        let (start_date, end_date) = transaction_date_range(chrono::Utc::now());
        let resp = self
            .http
            .get(endpoint)
            .query(&[
                ("start-date", start_date.as_str()),
                ("end-date", end_date.as_str()),
                ("version", "2"),
            ])
            .send()
            .await
            .map_err(|e| SimpleFinError::Http(e.without_url()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SimpleFinError::Http(e.without_url()))?;
        if !status.is_success() {
            return Err(SimpleFinError::Api {
                status: status.as_u16(),
                message: extract_error_message(&text),
            });
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| SimpleFinError::Parse(e.to_string()))?;
        if !v.get("accounts").is_some_and(Value::is_array) {
            return Err(SimpleFinError::Parse(
                "missing account list; saved balances were kept".into(),
            ));
        }
        Ok(parse_account_set(&v))
    }

    /// Validate the access URL by fetching accounts. Used right after claiming.
    pub async fn check(&self) -> Result<(), SimpleFinError> {
        self.fetch_accounts().await.map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split an access URL into the `/accounts` endpoint plus the Basic-auth username + password it
/// embeds. SimpleFIN access URLs look like `https://user:pass@host/simplefin`.
fn accounts_endpoint(access_url: &str) -> Result<reqwest::Url, SimpleFinError> {
    let mut url = secure_url(access_url.trim())?;
    if url.username().is_empty() || url.password().is_none_or(str::is_empty) {
        return Err(SimpleFinError::Url(
            "SimpleFIN app credentials are missing".into(),
        ));
    }
    url.set_path(&format!("{}/accounts", url.path().trim_end_matches('/')));
    // reqwest decodes URL credentials and moves them to the Basic Auth header.
    Ok(url)
}

/// Parse a numeric string ("100.23") or a bare JSON number into an `f64`.
fn parse_decimal(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => v.as_f64(),
    }
    .filter(|number| number.is_finite())
}

fn parse_account_set(v: &Value) -> SimpleFinAccountSet {
    let mut errors = Vec::new();
    let mut issues = Vec::new();
    // Bridge uses `errors` (strings); draft v2 uses `errlist` (objects with `msg`). Support both.
    for key in ["errors", "errlist"] {
        if let Some(arr) = v.get(key).and_then(Value::as_array) {
            for e in arr {
                if let Some(s) = e.as_str() {
                    errors.push(s.to_string());
                } else if let Some(msg) = e.get("msg").and_then(Value::as_str) {
                    errors.push(msg.to_string());
                    issues.push(SimpleFinIssue {
                        code: e
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("gen.")
                            .into(),
                        msg: msg.into(),
                        conn_id: e.get("conn_id").and_then(Value::as_str).map(str::to_owned),
                        account_id: e
                            .get("account_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    });
                }
                for message in &errors {
                    if !issues.iter().any(|issue| &issue.msg == message) {
                        issues.push(SimpleFinIssue {
                            code: "gen.".into(),
                            msg: message.clone(),
                            conn_id: None,
                            account_id: None,
                        });
                    }
                }
            }
        }
    }

    // Draft v2 separates connections; map conn_id -> name for institution lookup.
    let mut conn_names: HashMap<String, String> = HashMap::new();
    if let Some(arr) = v.get("connections").and_then(Value::as_array) {
        for c in arr {
            if let (Some(id), Some(name)) = (
                c.get("conn_id").and_then(Value::as_str),
                c.get("name").and_then(Value::as_str),
            ) {
                conn_names.insert(id.to_string(), name.to_string());
            }
        }
    }

    let mut accounts = Vec::new();
    if let Some(rows) = v.get("accounts").and_then(Value::as_array) {
        for row in rows {
            if let Some(account) = parse_account(row, &conn_names) {
                accounts.push(account);
            } else {
                let message =
                    "SimpleFIN returned an account without an ID; it could not be imported.";
                errors.push(message.into());
                issues.push(SimpleFinIssue {
                    code: "gen.".into(),
                    msg: message.into(),
                    conn_id: None,
                    account_id: None,
                });
            }
        }
    }

    let connections = conn_names
        .into_iter()
        .map(|(id, name)| SimpleFinConnection { id, name })
        .collect();
    SimpleFinAccountSet {
        accounts,
        errors,
        issues,
        connections,
    }
}

fn parse_account(v: &Value, conn_names: &HashMap<String, String>) -> Option<SimpleFinAccount> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Account")
        .to_string();
    let currency = v
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();

    // Institution: Bridge embeds `org.name`/`org.domain`; the protocol uses `conn_name` or a
    // `conn_id` resolved against the connections list.
    let institution = v
        .get("org")
        .and_then(|o| o.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            v.get("conn_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            v.get("conn_id")
                .and_then(Value::as_str)
                .and_then(|id| conn_names.get(id).cloned())
        })
        .or_else(|| {
            v.get("org")
                .and_then(|o| o.get("domain"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let holdings = v
        .get("holdings")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_holding).collect())
        .unwrap_or_default();

    let transactions = v
        .get("transactions")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_transaction).collect())
        .unwrap_or_default();

    Some(SimpleFinAccount {
        id,
        name,
        currency,
        balance: v.get("balance").and_then(parse_decimal),
        balance_date: v.get("balance-date").and_then(Value::as_i64),
        institution,
        connection_id: v
            .get("conn_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        holdings_reported: v.get("holdings").is_some_and(Value::is_array),
        holdings,
        transactions,
    })
}

/// Parse one SimpleFIN transaction. Requires an `id` and a numeric `amount`; the description
/// falls back to `payee` and then a placeholder, and the date prefers `posted` over
/// `transacted_at`.
fn parse_transaction(v: &Value) -> Option<SimpleFinTransaction> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let amount = v.get("amount").and_then(parse_decimal)?;
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .or_else(|| v.get("payee").and_then(Value::as_str))
        .unwrap_or("Transaction")
        .to_string();
    let posted = v
        .get("posted")
        .and_then(Value::as_i64)
        .or_else(|| v.get("transacted_at").and_then(Value::as_i64));
    let memo = v.get("memo").and_then(Value::as_str).map(str::to_string);
    Some(SimpleFinTransaction {
        id,
        amount,
        description,
        posted,
        memo,
    })
}

fn parse_holding(v: &Value) -> Option<SimpleFinHolding> {
    let symbol = v
        .get("symbol")
        .and_then(Value::as_str)
        .or_else(|| v.get("description").and_then(Value::as_str))?
        .to_string();
    Some(SimpleFinHolding {
        symbol,
        shares: v.get("shares").and_then(parse_decimal).unwrap_or(0.0),
        market_value: v.get("market_value").and_then(parse_decimal),
        cost_basis: v.get("cost_basis").and_then(parse_decimal),
        currency: v
            .get("currency")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Pull a human-readable message out of an error body, falling back to the raw text.
fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        // A top-level `errors`/`errlist` array, or a single `{ "msg": ... }` object.
        for key in ["errors", "errlist"] {
            if let Some(arr) = v.get(key).and_then(Value::as_array) {
                let msgs: Vec<String> = arr
                    .iter()
                    .filter_map(|e| {
                        e.as_str()
                            .map(str::to_string)
                            .or_else(|| e.get("msg").and_then(Value::as_str).map(str::to_string))
                    })
                    .collect();
                if !msgs.is_empty() {
                    return msgs.join("; ");
                }
            }
        }
        if let Some(msg) = v.get("msg").and_then(Value::as_str) {
            return msg.to_string();
        }
    }
    if text.trim().is_empty() {
        "no response body".to_string()
    } else {
        text.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_demo_setup_token() {
        // Base64 of https://bridge.simplefin.org/simplefin/claim/demo
        let token = "aHR0cHM6Ly9icmlkZ2Uuc2ltcGxlZmluLm9yZy9zaW1wbGVmaW4vY2xhaW0vZGVtbw==";
        assert_eq!(
            decode_setup_token(token).unwrap(),
            "https://bridge.simplefin.org/simplefin/claim/demo"
        );
    }

    #[test]
    fn rejects_garbage_and_empty_tokens() {
        assert!(decode_setup_token("").is_err());
        assert!(decode_setup_token("not base64!!!").is_err());
        // Valid base64 but not a URL.
        let not_a_url = base64::engine::general_purpose::STANDARD.encode("hello world");
        assert!(decode_setup_token(&not_a_url).is_err());
    }

    #[test]
    fn transaction_window_stays_inside_bridge_limit() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-31T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let (start, end) = transaction_date_range(now);
        let start = chrono::DateTime::from_timestamp(start.parse::<i64>().unwrap(), 0).unwrap();
        let end = chrono::DateTime::from_timestamp(end.parse::<i64>().unwrap(), 0).unwrap();

        assert_eq!(end, now);
        assert_eq!(end.signed_duration_since(start), chrono::Duration::days(89));
        assert!(end.signed_duration_since(start) < chrono::Duration::days(90));
    }

    #[test]
    fn splits_access_url_into_endpoint_and_credentials() {
        let endpoint =
            accounts_endpoint("https://abc123:secretpw@bridge.simplefin.org/simplefin").unwrap();
        assert_eq!(endpoint.path(), "/simplefin/accounts");
        assert_eq!(endpoint.username(), "abc123");
        assert_eq!(endpoint.password(), Some("secretpw"));
    }

    #[test]
    fn parses_bridge_account_with_org_and_holdings() {
        let v = json!({
            "errors": [],
            "accounts": [{
                "id": "act-1",
                "name": "Brokerage",
                "currency": "USD",
                "balance": "1234.56",
                "balance-date": 978366153i64,
                "org": { "name": "Wealthsimple", "domain": "wealthsimple.com" },
                "holdings": [{
                    "symbol": "AAPL",
                    "shares": "10",
                    "market_value": "1500.00",
                    "cost_basis": "1000.00",
                    "currency": "USD"
                }]
            }]
        });
        let set = parse_account_set(&v);
        assert!(set.errors.is_empty());
        assert_eq!(set.accounts.len(), 1);
        let acc = &set.accounts[0];
        assert_eq!(acc.id, "act-1");
        assert_eq!(acc.institution.as_deref(), Some("Wealthsimple"));
        assert_eq!(acc.balance, Some(1234.56));
        assert_eq!(acc.balance_date, Some(978366153));
        assert_eq!(acc.holdings.len(), 1);
        let h = &acc.holdings[0];
        assert_eq!(h.symbol, "AAPL");
        assert_eq!(h.shares, 10.0);
        assert_eq!(h.market_value, Some(1500.0));
        assert_eq!(h.cost_basis, Some(1000.0));
    }

    #[test]
    fn parses_protocol_account_with_connections_list() {
        let v = json!({
            "errlist": [{ "code": "con.auth", "msg": "Authentication required" }],
            "connections": [{ "conn_id": "CON-1", "name": "My Bank - Jill" }],
            "accounts": [{
                "id": "2930002",
                "name": "Savings",
                "conn_id": "CON-1",
                "currency": "CAD",
                "balance": "100.23",
                "balance-date": 978366153i64
            }]
        });
        let set = parse_account_set(&v);
        assert_eq!(set.errors, vec!["Authentication required".to_string()]);
        assert_eq!(set.accounts.len(), 1);
        let acc = &set.accounts[0];
        assert_eq!(acc.currency, "CAD");
        assert_eq!(acc.institution.as_deref(), Some("My Bank - Jill"));
        assert_eq!(acc.balance, Some(100.23));
        assert!(acc.holdings.is_empty());
    }

    #[test]
    fn account_missing_id_is_skipped() {
        let v = json!({ "accounts": [{ "name": "no id" }] });
        assert!(parse_account_set(&v).accounts.is_empty());
    }

    #[test]
    fn preserves_scoped_auth_errors_and_missing_holdings() {
        let set = parse_account_set(&json!({
            "connections": [{"conn_id": "one", "name": "Bank - personal"}],
            "errlist": [{"code": "con.auth", "msg": "Approve login", "conn_id": "one"}],
            "accounts": [{"id": "a", "conn_id": "one", "balance": "NaN"}]
        }));
        assert_eq!(set.issues[0].conn_id.as_deref(), Some("one"));
        assert_eq!(set.issues[0].code, "con.auth");
        assert_eq!(set.connections[0].name, "Bank - personal");
        assert_eq!(set.accounts[0].connection_id.as_deref(), Some("one"));
        assert!(!set.accounts[0].holdings_reported);
        assert!(set.accounts[0].balance.is_none());
    }

    #[test]
    fn basic_auth_decodes_credentials_without_putting_them_in_the_request_url() {
        let url =
            accounts_endpoint("https://sample:pass%3Aword@bridge.simplefin.org/simplefin").unwrap();
        let request = http_client().unwrap().get(url).build().unwrap();
        assert_eq!(request.url().username(), "");
        assert!(request.url().password().is_none());
        assert_eq!(
            request.headers()["authorization"],
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode("sample:pass:word")
            )
        );
    }

    #[test]
    fn parses_account_transactions_with_signed_amounts() {
        let v = json!({
            "accounts": [{
                "id": "act-1",
                "name": "Everyday Chequing",
                "currency": "CAD",
                "balance": "100.00",
                "transactions": [
                    {
                        "id": "txn-1",
                        "posted": 1700000000i64,
                        "amount": "-42.50",
                        "description": "GROCERY STORE"
                    },
                    {
                        // No `description` — falls back to `payee`; `transacted_at` dates it.
                        "id": "txn-2",
                        "transacted_at": 1700100000i64,
                        "amount": "2500.00",
                        "payee": "MICROSOFT PAYROLL",
                        "memo": "bi-weekly"
                    },
                    // Missing amount — skipped.
                    { "id": "txn-3", "description": "no amount" }
                ]
            }]
        });
        let set = parse_account_set(&v);
        let txns = &set.accounts[0].transactions;
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].id, "txn-1");
        assert_eq!(txns[0].amount, -42.50);
        assert_eq!(txns[0].description, "GROCERY STORE");
        assert_eq!(txns[0].posted, Some(1700000000));
        assert_eq!(txns[1].description, "MICROSOFT PAYROLL");
        assert_eq!(txns[1].amount, 2500.00);
        assert_eq!(txns[1].posted, Some(1700100000));
        assert_eq!(txns[1].memo.as_deref(), Some("bi-weekly"));
    }
}
