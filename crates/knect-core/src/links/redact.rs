//! Redaction helpers for connection evidence.
//!
//! Every value that reaches a protocol payload is either an endpoint built from
//! `scheme`/`host`/`port` with credentials removed, or the literal
//! `[REDACTED]` sentinel. Raw Secret and ConfigMap values are never formatted.

pub use knect_proto::redaction::REDACTED;

use super::dns::{parse_dns_reference, parse_ip_reference};
use super::uri::parse_uri_reference;

/// Builds a credential-free endpoint preview.
#[must_use]
pub fn endpoint_preview(scheme: Option<&str>, host: &str, port: Option<u16>) -> String {
    let host = host.trim();
    match (scheme, port) {
        (Some(scheme), Some(port)) => format!("{scheme}://{host}:{port}"),
        (Some(scheme), None) => format!("{scheme}://{host}"),
        (None, Some(port)) => format!("{host}:{port}"),
        (None, None) => host.to_owned(),
    }
}

/// Returns an endpoint preview for a recognizable value, or `[REDACTED]`.
///
/// This is the only function allowed to turn arbitrary scanned material into
/// a displayable value. It never echoes the raw input.
#[must_use]
pub fn value_preview(raw: &str) -> String {
    if let Some(uri) = parse_uri_reference(raw) {
        return uri.redacted;
    }
    if let Some((address, port)) = parse_ip_reference(raw) {
        return endpoint_preview(None, &address, port);
    }
    if let Some(dns) = parse_dns_reference(raw)
        && (dns.cluster_scoped || dns.port.is_some())
    {
        let host = dns.display_host();
        return endpoint_preview(None, &host, dns.port);
    }
    REDACTED.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_previews_keep_only_scheme_host_and_port() {
        assert_eq!(
            endpoint_preview(Some("redis"), "redis-master", Some(6379)),
            "redis://redis-master:6379"
        );
        assert_eq!(
            endpoint_preview(None, "redis-master", Some(6379)),
            "redis-master:6379"
        );
        assert_eq!(endpoint_preview(None, "redis-master", None), "redis-master");
    }

    #[test]
    fn sentinel_values_are_replaced_with_redacted_or_an_endpoint() {
        assert_eq!(value_preview("hunter2-knect-sentinel"), REDACTED);
        assert_eq!(
            value_preview("redis://:hunter2-knect-sentinel@redis-master:6379/0"),
            "redis://redis-master:6379"
        );
        assert!(!value_preview("redis://:hunter2-knect-sentinel@redis-master:6379").contains('@'));
        assert_eq!(value_preview("redis-master:6379"), "redis-master:6379");
        assert_eq!(
            value_preview("redis-master.knect-e2e.svc.cluster.local"),
            "redis-master.knect-e2e"
        );
        assert_eq!(value_preview("redis-master.knect-e2e"), REDACTED);
        assert_eq!(value_preview("example.com"), REDACTED);
        assert_eq!(value_preview("hello world"), REDACTED);
    }
}
