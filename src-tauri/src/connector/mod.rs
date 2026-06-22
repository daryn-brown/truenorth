use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod manual;
pub use manual::ManualConnector;

pub mod simplefin;
pub mod snaptrade;

// ---------------------------------------------------------------------------
// Domain types returned by connectors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorBalance {
    /// The connector's external account ID (connector_ref).
    pub account_ref: String,
    pub balance: f64,
    pub currency: String,
    /// ISO date YYYY-MM-DD
    pub snapshot_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorHolding {
    pub account_ref: String,
    pub symbol: String,
    pub quantity: f64,
    pub market_value: Option<f64>,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorTransaction {
    pub account_ref: String,
    /// ISO date YYYY-MM-DD
    pub txn_date: String,
    pub description: String,
    pub amount: f64,
    pub currency: String,
    pub external_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Auth error: {0}")]
    Auth(String),

    #[error("Not supported by this connector")]
    NotSupported,

    #[error("Connector error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// AccountConnector trait
//
// Each provider (Manual, SnapTrade, SimpleFIN, Questrade, …) implements this.
// The trait is object-safe so we can store `Box<dyn AccountConnector>` in a registry.
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AccountConnector: Send + Sync {
    /// Short identifier matching `connector_kind` in the `accounts` table.
    fn kind(&self) -> &'static str;

    /// Human-readable name for display.
    fn display_name(&self) -> &'static str;

    /// Pull the latest balance for each of the given external account refs.
    async fn fetch_balances(
        &self,
        account_refs: &[&str],
    ) -> Result<Vec<ConnectorBalance>, ConnectorError>;

    /// Pull current holdings (positions) for brokerage accounts.
    async fn fetch_holdings(
        &self,
        account_refs: &[&str],
    ) -> Result<Vec<ConnectorHolding>, ConnectorError>;

    /// Pull transactions since `since_date` (ISO date, inclusive) for one account.
    async fn fetch_transactions(
        &self,
        account_ref: &str,
        since_date: Option<&str>,
    ) -> Result<Vec<ConnectorTransaction>, ConnectorError>;
}

// ---------------------------------------------------------------------------
// ConnectorRegistry — resolved at runtime, built during app setup
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Arc;

pub struct ConnectorRegistry {
    connectors: HashMap<&'static str, Arc<dyn AccountConnector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            connectors: HashMap::new(),
        };
        // Always register the manual connector
        registry.register(Arc::new(ManualConnector));
        registry
    }

    pub fn register(&mut self, connector: Arc<dyn AccountConnector>) {
        self.connectors.insert(connector.kind(), connector);
    }

    pub fn get(&self, kind: &str) -> Option<Arc<dyn AccountConnector>> {
        self.connectors.get(kind).cloned()
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.connectors.keys().copied().collect()
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
