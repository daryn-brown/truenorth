//! Storage for connector secrets.
//!
//! SnapTrade has two sensitive values (`consumerKey`, per-user `userSecret`), plus the SimpleFIN
//! access URL and the Questrade refresh token. These are stored through
//! [`crate::db::secret_store`]. In "open mode" that means the local `secrets.json` file (no
//! keychain prompts); see the secret store module for the security tradeoff.
//!
//! Non-secret identifiers (`clientId`, `userId`, last-synced timestamp) are *not* stored here;
//! they live in the `app_settings` table.

use super::secret_store::{self, SecretStoreError};

/// Secret-store entry name for the SnapTrade API consumer key.
pub const SNAPTRADE_CONSUMER_KEY: &str = "snaptrade-consumer-key";
/// Secret-store entry name for the SnapTrade per-user secret.
pub const SNAPTRADE_USER_SECRET: &str = "snaptrade-user-secret";
/// Secret-store entry name for the SimpleFIN access URL. The access URL embeds HTTP Basic
/// credentials, so it's treated as a secret and stored alongside the SnapTrade secrets.
pub const SIMPLEFIN_ACCESS_URL: &str = "simplefin-access-url";
/// Secret-store entry name for the Questrade refresh token. Questrade rotates this on every use, so
/// it's the single durable secret for the direct Questrade connection (the short-lived access
/// token is never persisted).
pub const QUESTRADE_REFRESH_TOKEN: &str = "questrade-refresh-token";
/// Secret-store entry name for the GitHub Models personal access token (the AI advisor's key).
pub const GITHUB_MODELS_TOKEN: &str = "github-models-token";
/// Secret-store entry name for the user's Teller enrollments — a JSON array of `{ access_token,
/// institution, enrollment_id }`. Each Teller Connect enrollment yields one access token, and an
/// access token is useless without the matching client certificate, so it's treated as a secret.
pub const TELLER_ENROLLMENTS: &str = "teller-enrollments";
/// Secret-store entry name for the Teller client certificate (PEM). Required for Teller's
/// `development`/`production` environments (mTLS).
pub const TELLER_CERT_PEM: &str = "teller-certificate-pem";
/// Secret-store entry name for the Teller client private key (PEM) that pairs with the certificate.
pub const TELLER_KEY_PEM: &str = "teller-private-key-pem";

/// Store (or overwrite) a secret.
pub fn set_secret(account: &str, value: &str) -> Result<(), SecretStoreError> {
    secret_store::set(account, value)
}

/// Read a secret, returning `None` when no entry exists yet.
pub fn get_secret(account: &str) -> Result<Option<String>, SecretStoreError> {
    secret_store::get(account)
}

/// Delete a secret. Succeeds silently when the entry is already absent.
pub fn delete_secret(account: &str) -> Result<(), SecretStoreError> {
    secret_store::delete(account)
}
