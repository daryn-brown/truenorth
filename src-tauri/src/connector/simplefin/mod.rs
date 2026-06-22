//! SimpleFIN connector — read-only bank + investment balances via the SimpleFIN Bridge.
//!
//! Unlike SnapTrade, SimpleFIN needs no request signing or OAuth: the user pastes a one-time
//! **setup token** (Base64 of a claim URL), the app POSTs it once to claim a persistent
//! **access URL** with embedded HTTP Basic credentials, then GETs `/accounts`. The access URL is
//! the only secret and lives in the OS keychain. Sync orchestration (mapping accounts into the
//! local schema and writing balance snapshots) lives in `commands::simplefin`.

mod client;
pub use client::{
    claim_access_url, SimpleFinAccount, SimpleFinAccountSet, SimpleFinClient, SimpleFinError,
    SimpleFinHolding,
};
