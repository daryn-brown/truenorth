//! SnapTrade connector — read-only brokerage sync (Robinhood, Questrade, Wealthsimple, …).
//!
//! [`sign`] implements SnapTrade's request-signing scheme; [`client`] wraps the handful of
//! read endpoints TrueNorth uses. The sync orchestration (mapping SnapTrade accounts into the
//! local schema and writing balance snapshots) lives in `commands::snaptrade`.

pub mod sign;

mod client;
pub use client::{SnapAccount, SnapPosition, SnapTradeClient, SnapTradeError};
