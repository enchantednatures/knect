//! DNS and address parsing for connection heuristics.
//!
//! Only Kubernetes-looking names are accepted. External domains, free text,
//! and bare IP addresses are rejected here so the scanner cannot invent a link
//! from an arbitrary string. IP addresses are handled separately and only match
//! an existing `Endpoints` address.

use std::net::IpAddr;

/// Maximum accepted host name length; longer values are treated as free text.
const MAX_VALUE_LEN: usize = 2048;
/// Maximum Kubernetes label length.
const MAX_LABEL_LEN: usize = 63;

/// A parsed Kubernetes service-style DNS name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsReference {
    /// Short service or workload name (first label).
    pub name: String,
    /// Namespace when the name carried one.
    pub namespace: Option<String>,
    /// Optional port separated by `:`.
    pub port: Option<u16>,
    /// True for `name.namespace.svc[.cluster.local]` forms.
    pub cluster_scoped: bool,
}

impl DnsReference {
    /// Rebuilds the host without credentials, keeping a namespace when present.
    #[must_use]
    pub fn display_host(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("{}.{}", self.name, namespace),
            None => self.name.clone(),
        }
    }
}

/// Parses a DNS-style service reference.
///
/// Recognized forms (optionally suffixed with `:port`):
///
/// - `name`
/// - `name.namespace`
/// - `name.namespace.svc`
/// - `name.namespace.svc.cluster.local`
#[must_use]
pub fn parse_dns_reference(raw: &str) -> Option<DnsReference> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.len() > MAX_VALUE_LEN {
        return None;
    }
    if trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if trimmed.contains("://")
        || trimmed.contains('/')
        || trimmed.contains('@')
        || trimmed.contains('=')
        || trimmed.contains(',')
    {
        return None;
    }

    let (host, port) = split_port(trimmed)?;
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > MAX_VALUE_LEN || host.parse::<IpAddr>().is_ok() {
        return None;
    }

    let labels: Vec<String> = host.split('.').map(str::to_ascii_lowercase).collect();
    if labels.iter().any(|label| !valid_label(label)) {
        return None;
    }
    let count = labels.len();
    let (name, namespace, cluster_scoped) =
        if (count == 5 && labels[2] == "svc" && labels[3] == "cluster" && labels[4] == "local")
            || (count == 3 && labels[2] == "svc")
        {
            (labels[0].clone(), Some(labels[1].clone()), true)
        } else if count == 2 {
            (labels[0].clone(), Some(labels[1].clone()), false)
        } else if count == 1 {
            (labels[0].clone(), None, false)
        } else {
            return None;
        };

    Some(DnsReference {
        name,
        namespace,
        port,
        cluster_scoped,
    })
}

/// Parses an IP address reference with an optional port.
///
/// Only dotted-quad or IPv6 literals are accepted; hostnames are handled by
/// [`parse_dns_reference`].
#[must_use]
pub fn parse_ip_reference(raw: &str) -> Option<(String, Option<u16>)> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.len() > MAX_VALUE_LEN {
        return None;
    }
    if let Ok(address) = trimmed.parse::<IpAddr>() {
        return Some((address.to_string(), None));
    }
    let (host, port) = split_port(trimmed)?;
    let address: IpAddr = host.parse().ok()?;
    Some((address.to_string(), port))
}

fn split_port(value: &str) -> Option<(&str, Option<u16>)> {
    if value.matches(':').count() > 1 {
        return None;
    }
    match value.split_once(':') {
        Some((host, port_text)) => {
            let port: u16 = port_text.parse().ok()?;
            if port == 0 {
                return None;
            }
            Some((host, Some(port)))
        }
        None => Some((value, None)),
    }
}

fn valid_label(label: &str) -> bool {
    if label.is_empty() || label.len() > MAX_LABEL_LEN {
        return false;
    }
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let bytes = label.as_bytes();
    alphanumeric(bytes[0])
        && alphanumeric(*bytes.last().unwrap_or(&b'0'))
        && bytes
            .iter()
            .all(|byte| alphanumeric(*byte) || *byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_kubernetes_dns_forms() {
        let short = parse_dns_reference("redis-master").expect("short name");
        assert_eq!(short.name, "redis-master");
        assert_eq!(short.namespace, None);
        assert_eq!(short.port, None);
        assert!(!short.cluster_scoped);

        let namespaced = parse_dns_reference("redis-master.knect-e2e").expect("namespace");
        assert_eq!(namespaced.name, "redis-master");
        assert_eq!(namespaced.namespace.as_deref(), Some("knect-e2e"));
        assert!(!namespaced.cluster_scoped);

        let svc = parse_dns_reference("redis-master.knect-e2e.svc").expect("svc form");
        assert_eq!(svc.namespace.as_deref(), Some("knect-e2e"));
        assert!(svc.cluster_scoped);

        let cluster = parse_dns_reference("redis-master.knect-e2e.svc.cluster.local:6379")
            .expect("cluster form");
        assert_eq!(cluster.name, "redis-master");
        assert_eq!(cluster.namespace.as_deref(), Some("knect-e2e"));
        assert_eq!(cluster.port, Some(6379));
        assert!(cluster.cluster_scoped);
    }

    #[test]
    fn handles_case_and_trailing_dots() {
        let parsed = parse_dns_reference("Redis-Master.Knect-E2E.SVC.Cluster.Local.:5432")
            .expect("normalized");
        assert_eq!(parsed.name, "redis-master");
        assert_eq!(parsed.namespace.as_deref(), Some("knect-e2e"));
        assert_eq!(parsed.port, Some(5432));
    }

    #[test]
    fn rejects_external_names_and_free_text() {
        let two_labels = parse_dns_reference("example.com").expect("namespace-shaped name");
        assert_eq!(two_labels.name, "example");
        assert_eq!(two_labels.namespace.as_deref(), Some("com"));
        assert!(parse_dns_reference("api.example.com").is_none());
        assert!(parse_dns_reference("https://api.example.com/v1").is_none());
        assert!(parse_dns_reference("hello world").is_none());
        assert!(parse_dns_reference("10.0.0.10").is_none());
        assert!(parse_dns_reference("bad_host").is_none());
        assert!(parse_dns_reference("-leading.hyphen").is_none());
        assert!(parse_dns_reference("").is_none());
        assert!(parse_dns_reference("redis-master:0").is_none());
        assert!(parse_dns_reference("redis-master:99999").is_none());
    }

    #[test]
    fn ip_references_require_a_literal_address() {
        assert_eq!(
            parse_ip_reference("10.0.0.10:6379"),
            Some(("10.0.0.10".to_owned(), Some(6379)))
        );
        assert_eq!(
            parse_ip_reference("10.0.0.10"),
            Some(("10.0.0.10".to_owned(), None))
        );
        assert_eq!(parse_ip_reference("::1"), Some(("::1".to_owned(), None)));
        assert!(parse_ip_reference("redis-master:6379").is_none());
        assert!(parse_ip_reference("hello").is_none());
    }
}
