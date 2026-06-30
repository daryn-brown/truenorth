//! Teller connector — read-only US bank balances over a free Teller account.
//!
//! Teller authenticates with a client certificate (mTLS) plus a per-enrollment access token minted
//! by Teller Connect. The HTTP client and response parsing live in `client`; the sync orchestration
//! (mapping accounts into the local schema and writing balance snapshots) lives in
//! `commands::teller`.

mod client;
pub use client::{TellerAccount, TellerClient, TellerError};
