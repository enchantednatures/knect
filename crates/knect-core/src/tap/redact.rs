//! Header redaction for CloudEvent tap deliveries.

use std::collections::BTreeMap;

use knect_proto::redaction::REDACTED;

/// Maximum retained length of one callback header value.
pub(crate) const MAX_HEADER_VALUE: usize = 8_192;

/// Redacts credential-bearing headers while preserving diagnostic headers.
#[must_use]
pub fn redact_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let name = name.to_ascii_lowercase();
            let value = if sensitive_header(&name) {
                REDACTED.to_owned()
            } else {
                value.chars().take(MAX_HEADER_VALUE).collect()
            };
            (name, value)
        })
        .collect()
}

fn sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "x-knect-tap-token"
            | "x-csrf-token"
    ) || name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("api-key")
        || name.contains("apikey")
}
