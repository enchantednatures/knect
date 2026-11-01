//! URI parsing for backing-service connection strings.
//!
//! Only schemes in the recognized backing-service set produce a reference.
//! User information (username/password), path, and query are discarded
//! immediately; only a credential-free `scheme://host[:port]` preview survives.

use url::Url;

use super::dns::parse_dns_reference;
use super::ports::{PortClass, class_for_scheme};

/// Maximum accepted URI length.
const MAX_URI_LEN: usize = 2048;

/// A parsed backing-service URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriReference {
    /// Canonical scheme (`postgresql` is normalized to `postgres`).
    pub scheme: String,
    /// Host without user information.
    pub host: String,
    /// Namespace when the host uses a Kubernetes DNS form.
    pub namespace: Option<String>,
    /// Effective port: explicit, or the scheme default.
    pub port: u16,
    /// Protocol class for the scheme.
    pub class: PortClass,
    /// Credential-free preview suitable for evidence.
    pub redacted: String,
}

/// Parses a recognized backing-service URI.
#[must_use]
pub fn parse_uri_reference(raw: &str) -> Option<UriReference> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.len() > MAX_URI_LEN {
        return None;
    }
    let normalized = trimmed
        .strip_prefix("jdbc:")
        .or_else(|| trimmed.strip_prefix("JDBC:"))
        .unwrap_or(trimmed);
    if normalized.chars().any(char::is_whitespace) {
        return None;
    }
    let url = Url::parse(normalized).ok()?;
    let class = class_for_scheme(url.scheme())?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let port = url.port().unwrap_or_else(|| class.port());
    if port == 0 {
        return None;
    }
    let dns = parse_dns_reference(&host);
    let namespace = dns.as_ref().and_then(|dns| dns.namespace.clone());
    let display_host = dns
        .as_ref()
        .map_or_else(|| host.clone(), |dns| dns.display_host());
    let redacted = format!("{}://{}:{}", class.scheme(), display_host, port);
    Some(UriReference {
        scheme: class.scheme().to_owned(),
        host,
        namespace,
        port,
        class,
        redacted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_backing_service_schemes() {
        let redis = parse_uri_reference("redis://redis-master:6379/0").expect("redis");
        assert_eq!(redis.scheme, "redis");
        assert_eq!(redis.host, "redis-master");
        assert_eq!(redis.port, 6379);
        assert_eq!(redis.redacted, "redis://redis-master:6379");

        let postgres = parse_uri_reference("postgresql://order:secret@postgres-e2e-rw:5432/orders")
            .expect("postgresql");
        assert_eq!(postgres.scheme, "postgres");
        assert_eq!(postgres.namespace, None);
        assert_eq!(postgres.redacted, "postgres://postgres-e2e-rw:5432");

        let mysql = parse_uri_reference("mysql://app:pw@mysql-e2e:3306/app").expect("mysql");
        assert_eq!(mysql.class, PortClass::Mysql);

        let amqp = parse_uri_reference("amqp://guest:guest@rabbitmq:5672/%2f").expect("amqp");
        assert_eq!(amqp.class, PortClass::Amqp);

        let kafka = parse_uri_reference("kafka://kafka-bootstrap.knect-e2e.svc.cluster.local:9092")
            .expect("kafka");
        assert_eq!(kafka.namespace.as_deref(), Some("knect-e2e"));
    }

    #[test]
    fn credentials_never_reach_the_preview() {
        let parsed = parse_uri_reference("redis://:hunter2-knect-sentinel@redis-master:6379/0")
            .expect("redis");
        assert!(!parsed.redacted.contains("hunter2-knect-sentinel"));
        assert!(!parsed.redacted.contains('@'));
        assert_eq!(parsed.redacted, "redis://redis-master:6379");
    }

    #[test]
    fn default_ports_are_applied() {
        let parsed = parse_uri_reference("redis://redis-master").expect("redis");
        assert_eq!(parsed.port, 6379);
        assert_eq!(parsed.redacted, "redis://redis-master:6379");
    }

    #[test]
    fn rejects_unknown_schemes_and_free_text() {
        assert!(parse_uri_reference("https://api.example.com/v1").is_none());
        assert!(parse_uri_reference("http://order-processor.knect-e2e").is_none());
        assert!(parse_uri_reference("redis://redis master").is_none());
        assert!(parse_uri_reference("hello world").is_none());
        assert!(parse_uri_reference("redis-master:6379").is_none());
        assert!(parse_uri_reference("").is_none());
    }

    #[test]
    fn jdbc_prefix_is_tolerated() {
        let parsed = parse_uri_reference("jdbc:postgresql://postgres-e2e-rw:5432/orders")
            .expect("jdbc postgres");
        assert_eq!(parsed.scheme, "postgres");
        assert_eq!(parsed.host, "postgres-e2e-rw");
    }
}
