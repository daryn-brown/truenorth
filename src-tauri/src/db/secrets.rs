//! OS-keychain storage for connector secrets.
//!
//! SnapTrade has two sensitive values that must never touch disk in the clear:
//! the API `consumerKey` and the per-user `userSecret`. Both live in the OS keychain
//! (macOS Keychain / Windows Credential Manager) under the same service identifier as the
//! database encryption key — see [`crate::db::crypto`].
//!
//! Non-secret identifiers (`clientId`, `userId`, last-synced timestamp) are *not* stored here;
//! they live in the `app_settings` table.

use keyring::{Entry, Error as KeyringError};

use super::crypto::KEY_SERVICE;

/// Keychain entry name for the SnapTrade API consumer key.
pub const SNAPTRADE_CONSUMER_KEY: &str = "snaptrade-consumer-key";
/// Keychain entry name for the SnapTrade per-user secret.
pub const SNAPTRADE_USER_SECRET: &str = "snaptrade-user-secret";
/// Keychain entry name for the SimpleFIN access URL. The access URL embeds HTTP Basic
/// credentials, so it's treated as a secret and stored alongside the SnapTrade secrets.
pub const SIMPLEFIN_ACCESS_URL: &str = "simplefin-access-url";

fn entry(account: &str) -> Result<Entry, KeyringError> {
    Entry::new(KEY_SERVICE, account)
}

/// Store (or overwrite) a secret.
pub fn set_secret(account: &str, value: &str) -> Result<(), KeyringError> {
    entry(account)?.set_password(value)
}

/// Read a secret, returning `None` when no entry exists yet.
pub fn get_secret(account: &str) -> Result<Option<String>, KeyringError> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Delete a secret. Succeeds silently when the entry is already absent.
pub fn delete_secret(account: &str) -> Result<(), KeyringError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}
