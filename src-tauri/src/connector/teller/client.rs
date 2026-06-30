//! Minimal Teller API client — read-only account balances.
//!
//! Teller (<https://teller.io>) is a free-for-personal-use US bank aggregator. It authenticates
//! API calls with **two** layers:
//!
//! 1. **mTLS** — Teller issues a client certificate (a cert + private key pair). It is *required*
//!    for the `development` and `production` environments and optional in `sandbox`. We present it
//!    with [`reqwest::Identity::from_pem`], which the project's `rustls-tls` backend supports.
//! 2. **Access token** — minted when the end-user completes Teller Connect. It is sent via HTTP
//!    Basic auth as the *username* with an empty password.
//!
//! The flow we need is small: `GET /accounts` to list accounts, then
//! `GET /accounts/:id/balances` per account. Balances feed the net-worth pipeline. Responses are
//! parsed defensively from `serde_json::Value` because institutions vary in which fields they fill.

use serde_json::Value;
use thiserror::Error;

const DEFAULT_BASE_URL: &str = "https://api.teller.io";

#[derive(Debug, Error)]
pub enum TellerError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Teller API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Invalid Teller client certificate: {0}")]
    Certificate(String),

    #[error("Unexpected Teller response: {0}")]
    Parse(String),
}

impl TellerError {
    /// True when Teller rejected our request because the token/certificate is wrong (HTTP 401/403).
    /// Used to surface a clear "re-link" message instead of a raw status code.
    pub fn is_auth(&self) -> bool {
        matches!(self, TellerError::Api { status, .. } if *status == 401 || *status == 403)
    }
}

/// One account as reported by Teller's `/accounts` endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct TellerAccount {
    /// Teller account id (`acc_…`) — stored as `connector_ref`.
    pub id: String,
    pub name: String,
    /// Teller account class: `depository` (asset) or `credit` (liability).
    pub kind: String,
    /// Finer-grained type, e.g. `checking`, `savings`, `credit_card`.
    pub subtype: Option<String>,
    /// ISO-4217 currency. Teller is USD-only today, but parse it rather than assume.
    pub currency: String,
    pub last_four: Option<String>,
    pub institution: Option<String>,
    pub enrollment_id: Option<String>,
    /// `open` or `closed`. Closed accounts are skipped during sync.
    pub status: Option<String>,
}

/// The balances reported for one account. At least one of `ledger`/`available` is always present.
#[derive(Debug, Clone, PartialEq)]
pub struct TellerBalance {
    pub account_id: String,
    /// Total funds in the account (the figure that feeds net worth).
    pub ledger: Option<f64>,
    /// Ledger net of pending in/outflows.
    pub available: Option<f64>,
}

impl TellerBalance {
    /// The balance that feeds net worth: prefer the ledger (total funds), else the available
    /// balance. For credit accounts this is the amount owed (a positive number from Teller — the
    /// caller is responsible for giving liabilities a negative sign).
    pub fn primary(&self) -> Option<f64> {
        self.ledger.or(self.available)
    }
}

/// A thin Teller API client bound to one access token (one enrollment).
pub struct TellerClient {
    access_token: String,
    base_url: String,
    http: reqwest::Client,
}

impl TellerClient {
    /// Build a client for one access token. When `identity_pem` is `Some`, the bytes must contain a
    /// PEM-encoded certificate **and** private key; they are presented for mTLS (required in
    /// `development`/`production`). `None` only works in `sandbox`.
    pub fn new(
        access_token: impl Into<String>,
        identity_pem: Option<&[u8]>,
    ) -> Result<Self, TellerError> {
        let mut builder = reqwest::Client::builder();
        if let Some(pem) = identity_pem {
            let identity = reqwest::Identity::from_pem(pem)
                .map_err(|e| TellerError::Certificate(e.to_string()))?;
            builder = builder.identity(identity);
        }
        let http = builder.build()?;
        Ok(Self {
            access_token: access_token.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
        })
    }

    /// Override the API base URL (used by tests).
    #[cfg(test)]
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    async fn get(&self, path: &str) -> Result<Value, TellerError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            // Teller wants the access token as the Basic-auth username, empty password.
            .basic_auth(&self.access_token, None::<&str>)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(TellerError::Api {
                status: status.as_u16(),
                message: extract_error_message(&text),
            });
        }
        serde_json::from_str(&text).map_err(|e| TellerError::Parse(e.to_string()))
    }

    /// List the accounts this token can access.
    pub async fn fetch_accounts(&self) -> Result<Vec<TellerAccount>, TellerError> {
        let v = self.get("/accounts").await?;
        Ok(parse_accounts(&v))
    }

    /// Fetch the live balances for one account.
    pub async fn fetch_balance(&self, account_id: &str) -> Result<TellerBalance, TellerError> {
        let v = self.get(&format!("/accounts/{account_id}/balances")).await?;
        parse_balance(&v)
            .ok_or_else(|| TellerError::Parse("balances response had no account_id".into()))
    }

    /// Validate the token (and certificate) by listing accounts. Returns them so callers can grab
    /// the institution name right after linking.
    pub async fn check(&self) -> Result<Vec<TellerAccount>, TellerError> {
        self.fetch_accounts().await
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse a numeric string ("100.23") or a bare JSON number into an `f64`.
fn parse_decimal(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => v.as_f64(),
    }
}

/// Teller returns `/accounts` as a bare JSON array. We also accept an `{ "accounts": [...] }`
/// wrapper defensively.
fn parse_accounts(v: &Value) -> Vec<TellerAccount> {
    let arr = v
        .as_array()
        .or_else(|| v.get("accounts").and_then(Value::as_array));
    arr.map(|items| items.iter().filter_map(parse_account).collect())
        .unwrap_or_default()
}

fn parse_account(v: &Value) -> Option<TellerAccount> {
    let id = v.get("id").and_then(Value::as_str)?.to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Account")
        .to_string();
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("depository")
        .to_string();
    let subtype = v.get("subtype").and_then(Value::as_str).map(str::to_string);
    let currency = v
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();
    let last_four = v
        .get("last_four")
        .and_then(Value::as_str)
        .map(str::to_string);
    let institution = v
        .get("institution")
        .and_then(|i| i.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let enrollment_id = v
        .get("enrollment_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let status = v.get("status").and_then(Value::as_str).map(str::to_string);

    Some(TellerAccount {
        id,
        name,
        kind,
        subtype,
        currency,
        last_four,
        institution,
        enrollment_id,
        status,
    })
}

fn parse_balance(v: &Value) -> Option<TellerBalance> {
    let account_id = v.get("account_id").and_then(Value::as_str)?.to_string();
    Some(TellerBalance {
        account_id,
        ledger: v.get("ledger").and_then(parse_decimal),
        available: v.get("available").and_then(parse_decimal),
    })
}

/// Teller error bodies look like `{ "error": { "code": "...", "message": "..." } }`.
fn extract_error_message(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
        {
            return msg.to_string();
        }
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
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
    fn parses_accounts_array_with_institution_and_subtype() {
        let v = json!([
            {
                "id": "acc_1",
                "name": "Everyday Checking",
                "type": "depository",
                "subtype": "checking",
                "currency": "USD",
                "last_four": "1234",
                "status": "open",
                "enrollment_id": "enr_1",
                "institution": { "id": "chase", "name": "Chase" }
            },
            {
                "id": "acc_2",
                "name": "Sapphire Card",
                "type": "credit",
                "subtype": "credit_card",
                "currency": "USD",
                "institution": { "id": "chase", "name": "Chase" }
            }
        ]);
        let accounts = parse_accounts(&v);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].id, "acc_1");
        assert_eq!(accounts[0].kind, "depository");
        assert_eq!(accounts[0].subtype.as_deref(), Some("checking"));
        assert_eq!(accounts[0].institution.as_deref(), Some("Chase"));
        assert_eq!(accounts[0].enrollment_id.as_deref(), Some("enr_1"));
        assert_eq!(accounts[1].kind, "credit");
    }

    #[test]
    fn accepts_accounts_wrapper_object() {
        let v = json!({ "accounts": [{ "id": "acc_1", "name": "A", "type": "depository" }] });
        assert_eq!(parse_accounts(&v).len(), 1);
    }

    #[test]
    fn account_without_id_is_skipped() {
        let v = json!([{ "name": "no id", "type": "depository" }]);
        assert!(parse_accounts(&v).is_empty());
    }

    #[test]
    fn parses_balances_as_strings_and_prefers_ledger() {
        let v = json!({ "account_id": "acc_1", "ledger": "1234.56", "available": "1200.00" });
        let b = parse_balance(&v).unwrap();
        assert_eq!(b.account_id, "acc_1");
        assert_eq!(b.ledger, Some(1234.56));
        assert_eq!(b.available, Some(1200.00));
        assert_eq!(b.primary(), Some(1234.56));
    }

    #[test]
    fn balance_falls_back_to_available_when_ledger_absent() {
        let v = json!({ "account_id": "acc_1", "available": "50.25" });
        let b = parse_balance(&v).unwrap();
        assert_eq!(b.ledger, None);
        assert_eq!(b.primary(), Some(50.25));
    }

    #[test]
    fn balance_without_account_id_is_none() {
        assert!(parse_balance(&json!({ "ledger": "1.00" })).is_none());
    }

    #[test]
    fn extracts_nested_error_message() {
        let body = r#"{ "error": { "code": "unauthorized", "message": "bad token" } }"#;
        assert_eq!(extract_error_message(body), "bad token");
    }
}
