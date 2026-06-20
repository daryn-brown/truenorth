use async_trait::async_trait;

use super::{
    AccountConnector, ConnectorBalance, ConnectorError, ConnectorHolding, ConnectorTransaction,
};

/// The `ManualConnector` is a no-op connector.
///
/// All balance and holding data is entered directly by the user through the UI
/// and stored in SQLite. This connector exists so that manual accounts participate
/// in the same `AccountConnector` abstraction as SnapTrade, SimpleFIN, etc.
pub struct ManualConnector;

#[async_trait]
impl AccountConnector for ManualConnector {
    fn kind(&self) -> &'static str {
        "manual"
    }

    fn display_name(&self) -> &'static str {
        "Manual Entry"
    }

    async fn fetch_balances(
        &self,
        _account_refs: &[&str],
    ) -> Result<Vec<ConnectorBalance>, ConnectorError> {
        // Manual accounts have no remote source; balances are user-supplied.
        Ok(vec![])
    }

    async fn fetch_holdings(
        &self,
        _account_refs: &[&str],
    ) -> Result<Vec<ConnectorHolding>, ConnectorError> {
        Ok(vec![])
    }

    async fn fetch_transactions(
        &self,
        _account_ref: &str,
        _since_date: Option<&str>,
    ) -> Result<Vec<ConnectorTransaction>, ConnectorError> {
        Ok(vec![])
    }
}
