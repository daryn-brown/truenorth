//! Questrade connector — read-only balances + holdings via the official Questrade REST API.
//!
//! Unlike SimpleFIN (banks) and SnapTrade (aggregated brokerages), this talks directly to
//! Questrade's free personal API: the user pastes a manual-authorization **refresh token**, which
//! we exchange for a short-lived access token and the account-specific data host, then read
//! accounts, balances, and positions. The refresh token rotates on every use and is the only
//! durable secret (stored in the OS keychain). Sync orchestration lives in `commands::questrade`.

mod client;
pub use client::{
    refresh_access_token, QuestradeAccount, QuestradeBalance, QuestradeClient, QuestradeError,
    QuestradePosition,
};
