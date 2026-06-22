//! SnapTrade request signing.
//!
//! SnapTrade authenticates direct (non-SDK) API calls with a `Signature` header. Per
//! <https://docs.snaptrade.com/docs/request-signatures> the signature is:
//!
//! 1. Build a payload object with exactly three keys: `content` (the JSON request body, or
//!    `null` for bodyless/GET requests), `path` (request path including `/api/v1`, excluding
//!    the query string), and `query` (the raw query string, excluding the leading `?`).
//! 2. Serialize it to **canonical JSON**: object keys sorted alphabetically at every level,
//!    no insignificant whitespace, UTF-8.
//! 3. HMAC-SHA256 the canonical string with the `consumerKey`.
//! 4. Base64-encode the digest and send it in the `Signature` header.
//!
//! The canonical serializer here is deliberately independent of `serde_json`'s map ordering
//! (which becomes insertion-ordered if the `preserve_order` feature is enabled anywhere in
//! the dependency tree) — it always sorts keys itself.

use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Compute the SnapTrade `Signature` header value:
/// `base64( HMAC-SHA256( consumer_key, canonical_json({content, path, query}) ) )`.
pub fn signature(consumer_key: &str, content: Option<&Value>, path: &str, query: &str) -> String {
    let canonical = canonical_signing_string(content, path, query);
    // HMAC accepts a key of any length, so `new_from_slice` cannot fail here.
    let mut mac =
        HmacSha256::new_from_slice(consumer_key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(canonical.as_bytes());
    let digest = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// The exact canonical JSON string that gets signed. Exposed for testing.
pub fn canonical_signing_string(content: Option<&Value>, path: &str, query: &str) -> String {
    let null = Value::Null;
    let content = content.unwrap_or(&null);

    // Top-level keys emitted in sorted order: content < path < query.
    let mut out = String::new();
    out.push_str("{\"content\":");
    write_canonical(content, &mut out);
    out.push_str(",\"path\":");
    write_json_string(path, &mut out);
    out.push_str(",\"query\":");
    write_json_string(query, &mut out);
    out.push('}');
    out
}

/// Recursively write `value` as canonical JSON (sorted object keys, no whitespace).
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(key, out);
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

/// Serialize a `&str` as a JSON string literal (quoted + escaped) using `serde_json` so the
/// escaping exactly matches what the server expects.
fn write_json_string(s: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(s).expect("serializing a string is infallible"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_string_matches_documented_example() {
        // From https://docs.snaptrade.com/docs/request-signatures
        //   POST /api/v1/snapTrade/registerUser?clientId=PASSIVTEST&timestamp=1635790389
        //   body: {"userId":"new_user_123"}
        let content = json!({ "userId": "new_user_123" });
        let canonical = canonical_signing_string(
            Some(&content),
            "/api/v1/snapTrade/registerUser",
            "clientId=PASSIVTEST&timestamp=1635790389",
        );
        assert_eq!(
            canonical,
            r#"{"content":{"userId":"new_user_123"},"path":"/api/v1/snapTrade/registerUser","query":"clientId=PASSIVTEST&timestamp=1635790389"}"#
        );
    }

    #[test]
    fn bodyless_request_uses_null_content() {
        let canonical = canonical_signing_string(
            None,
            "/api/v1/accounts",
            "clientId=ABC&timestamp=1&userId=u&userSecret=s",
        );
        assert_eq!(
            canonical,
            r#"{"content":null,"path":"/api/v1/accounts","query":"clientId=ABC&timestamp=1&userId=u&userSecret=s"}"#
        );
    }

    #[test]
    fn object_keys_are_sorted_at_every_level() {
        // Deliberately out-of-order keys; canonical form must sort them.
        let content = json!({
            "zeta": 1,
            "alpha": { "yankee": true, "bravo": [3, 2, 1] },
        });
        let canonical = canonical_signing_string(Some(&content), "/p", "q=1");
        assert_eq!(
            canonical,
            r#"{"content":{"alpha":{"bravo":[3,2,1],"yankee":true},"zeta":1},"path":"/p","query":"q=1"}"#
        );
    }

    #[test]
    fn signature_matches_reference_hmac_vector() {
        // Reference value independently computed with:
        //   printf '%s' 'message' | openssl dgst -sha256 -hmac 'secretkey' -binary | base64
        // This pins the hmac/sha2/base64 wiring to a known-good output.
        let mut mac = HmacSha256::new_from_slice(b"secretkey").unwrap();
        mac.update(b"message");
        let got = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert_eq!(got, "XD4vVt6UEQaPZ17zL/oSc1IQucv+4rpSE2ejlVM0o0M=");
    }

    #[test]
    fn signature_is_deterministic_and_nonempty() {
        let content = json!({ "userId": "abc" });
        let a = signature(
            "consumer-key",
            Some(&content),
            "/api/v1/x",
            "clientId=c&timestamp=1",
        );
        let b = signature(
            "consumer-key",
            Some(&content),
            "/api/v1/x",
            "clientId=c&timestamp=1",
        );
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // A different key must produce a different signature.
        let c = signature(
            "other-key",
            Some(&content),
            "/api/v1/x",
            "clientId=c&timestamp=1",
        );
        assert_ne!(a, c);
    }
}
