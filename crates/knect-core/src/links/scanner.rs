//! Store scanning core shared by the workload and binding scanners.

use std::net::IpAddr;

use base64::Engine as _;
use knect_proto::ResourceId;
use knect_proto::graph::EdgeKind;
use knect_proto::links::{Evidence, EvidenceKind, LinkVia};
use knect_proto::redaction::REDACTED;

use crate::correlate::CorrelationContext;
use crate::graph::support::{field, reference_id};
use crate::links::candidate::{LinkCandidate, LinkScan, UnresolvedTarget};
use crate::links::dns;
use crate::links::index::{Lookup, TargetIndex};
use crate::links::ports;
use crate::links::redact::endpoint_preview;
use crate::links::uri;

/// Maximum material value length decoded for link heuristics.
const MAX_MATERIAL_LEN: usize = 16 * 1024;
/// Maximum accepted free-form value length.
const MAX_VALUE_LEN: usize = 2048;

/// Reference style for a Secret or ConfigMap use.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReferenceStyle {
    pub(super) value_via: LinkVia,
    pub(super) evidence_kind: EvidenceKind,
    pub(super) confidence: f64,
}

impl ReferenceStyle {
    pub(super) const ENV_SECRET: Self = Self {
        value_via: LinkVia::Env,
        evidence_kind: EvidenceKind::SecretKey,
        confidence: 0.95,
    };
    pub(super) const ENV_FROM_SECRET: Self = Self {
        value_via: LinkVia::Env,
        evidence_kind: EvidenceKind::EnvFrom,
        confidence: 0.9,
    };
    pub(super) const VOLUME_SECRET: Self = Self {
        value_via: LinkVia::Secret,
        evidence_kind: EvidenceKind::Field,
        confidence: 0.85,
    };
    pub(super) const ENV_CONFIGMAP: Self = Self {
        value_via: LinkVia::ConfigMap,
        evidence_kind: EvidenceKind::ConfigMapKey,
        confidence: 0.9,
    };
    pub(super) const ENV_FROM_CONFIGMAP: Self = Self {
        value_via: LinkVia::ConfigMap,
        evidence_kind: EvidenceKind::EnvFrom,
        confidence: 0.85,
    };
    pub(super) const VOLUME_CONFIGMAP: Self = Self {
        value_via: LinkVia::ConfigMap,
        evidence_kind: EvidenceKind::Field,
        confidence: 0.8,
    };
}

/// Mutable scanner state for one correlation pass.
pub(super) struct Scanner<'a> {
    pub(super) ctx: &'a CorrelationContext,
    pub(super) index: TargetIndex,
    pub(super) scan: LinkScan,
}

impl<'a> Scanner<'a> {
    /// Creates a scanner over one correlation context.
    pub(super) fn new(ctx: &'a CorrelationContext) -> Self {
        Self {
            ctx,
            index: TargetIndex::build(&ctx.store),
            scan: LinkScan::new(),
        }
    }

    /// Runs every scan stage and returns the deduplicated result.
    pub(super) fn run(mut self) -> LinkScan {
        self.scan_bindings();
        self.scan_workloads();
        self.scan
    }

    /// Resolves a typed cross-resource reference against the store.
    pub(super) fn resolve_reference(
        &self,
        reference: &serde_json::Value,
        default_namespace: Option<&str>,
    ) -> Option<ResourceId> {
        let id = reference_id(reference, default_namespace)?;
        self.ctx.store.contains(&id).then_some(id)
    }

    /// Resolves a core Secret or ConfigMap by name in the source namespace.
    pub(super) fn resolve_named(
        &self,
        api_version: &str,
        kind: &str,
        source: &ResourceId,
        name: &str,
    ) -> Option<ResourceId> {
        let namespace = source.namespace.clone()?;
        let id = ResourceId::namespaced(api_version, kind, namespace, name);
        self.ctx.store.contains(&id).then_some(id)
    }

    /// Records an unresolved named reference as a warning.
    pub(super) fn unresolved_named(
        &mut self,
        source: &ResourceId,
        name: &str,
        namespace: Option<String>,
        preview: &str,
        via: LinkVia,
    ) {
        self.scan
            .unresolved(source, UnresolvedTarget::new(name, namespace, preview), via);
    }

    /// Adds a `UsesSecret` or `UsesConfigMap` candidate.
    pub(super) fn add_use_link(
        &mut self,
        source: &ResourceId,
        target: &ResourceId,
        via: LinkVia,
        kind: EdgeKind,
        confidence: f64,
        evidence: Evidence,
    ) {
        self.scan.add(LinkCandidate {
            source: source.clone(),
            target: target.clone(),
            via,
            kind,
            confidence,
            evidence: vec![evidence],
        });
    }

    /// Links a source to a Secret, or warns when it is absent.
    pub(super) fn link_secret(
        &mut self,
        source: &ResourceId,
        name: &str,
        key: Option<&str>,
        style: ReferenceStyle,
        context: &str,
        path: &str,
    ) {
        let Some(secret_id) = self.resolve_named("v1", "Secret", source, name) else {
            let preview = format!("Secret/{name}");
            self.unresolved_named(
                source,
                name,
                source.namespace.clone(),
                &preview,
                LinkVia::Secret,
            );
            return;
        };
        let label = reference_label("secret", name, key);
        let evidence = Evidence {
            kind: style.evidence_kind,
            description: format!("{context} references {label}"),
            path: Some(knect_proto::FieldPath::new(path)),
            value_redacted: Some(REDACTED.to_owned()),
        };
        self.add_use_link(
            source,
            &secret_id,
            LinkVia::Secret,
            EdgeKind::UsesSecret,
            style.confidence,
            evidence,
        );
        let description = format!("{context} {label}");
        self.scan_secret_values(source, &secret_id, style.value_via, &description, path);
    }

    /// Links a source to a ConfigMap, or warns when it is absent.
    pub(super) fn link_configmap(
        &mut self,
        source: &ResourceId,
        name: &str,
        key: Option<&str>,
        style: ReferenceStyle,
        context: &str,
        path: &str,
    ) {
        let Some(config_map_id) = self.resolve_named("v1", "ConfigMap", source, name) else {
            let preview = format!("ConfigMap/{name}");
            self.unresolved_named(
                source,
                name,
                source.namespace.clone(),
                &preview,
                LinkVia::ConfigMap,
            );
            return;
        };
        let label = reference_label("config map", name, key);
        let evidence = Evidence {
            kind: style.evidence_kind,
            description: format!("{context} references {label}"),
            path: Some(knect_proto::FieldPath::new(path)),
            value_redacted: Some(REDACTED.to_owned()),
        };
        self.add_use_link(
            source,
            &config_map_id,
            LinkVia::ConfigMap,
            EdgeKind::UsesConfigMap,
            style.confidence,
            evidence,
        );
        let description = format!("{context} {label}");
        self.scan_configmap_values(
            source,
            &config_map_id,
            key,
            style.value_via,
            &description,
            path,
        );
    }

    /// Scans every retained Secret value, discarding raw material immediately.
    pub(super) fn scan_secret_values(
        &mut self,
        source: &ResourceId,
        secret_id: &ResourceId,
        via: LinkVia,
        description: &str,
        path: &str,
    ) {
        let Some(material) = self.ctx.secret_material.get(secret_id) else {
            return;
        };
        for (entry, value) in material.entries() {
            if value.is_empty() || value.len() > MAX_MATERIAL_LEN {
                continue;
            }
            let key = entry.rsplit_once('/').map_or(entry, |(_field, key)| key);
            let Some(raw) = decode_secret_entry(entry, value.expose()) else {
                continue;
            };
            let evidence_description = format!("{description} key {key}");
            self.add_connection(
                source,
                &raw,
                via,
                EvidenceKind::SecretKey,
                &evidence_description,
                path,
            );
        }
    }

    /// Scans ConfigMap values without ever retaining or emitting the text.
    pub(super) fn scan_configmap_values(
        &mut self,
        source: &ResourceId,
        config_map_id: &ResourceId,
        only_key: Option<&str>,
        via: LinkVia,
        description: &str,
        path: &str,
    ) {
        let Some(object) = self.ctx.store.get(config_map_id) else {
            return;
        };
        let Some(data) = field(&object, &["data"]).and_then(serde_json::Value::as_object) else {
            return;
        };
        for (key, value) in data {
            if only_key.is_some_and(|wanted| wanted != key) {
                continue;
            }
            let Some(raw) = value.as_str() else {
                continue;
            };
            let evidence_description = format!("{description} key {key}");
            self.add_connection(
                source,
                raw,
                via,
                EvidenceKind::ConfigMapKey,
                &evidence_description,
                path,
            );
        }
    }

    /// Classifies a raw value and adds a resolved connection candidate.
    pub(super) fn add_connection(
        &mut self,
        source: &ResourceId,
        raw: &str,
        via: LinkVia,
        base_kind: EvidenceKind,
        description: &str,
        path: &str,
    ) {
        let Some(connection) = classify_connection(raw) else {
            return;
        };
        let lookup = if connection.is_ip {
            self.index
                .resolve_address(&connection.name, source.namespace.as_deref())
        } else {
            self.index.resolve_service(
                &connection.name,
                connection.namespace.as_deref(),
                source.namespace.as_deref(),
            )
        };
        let evidence = connection_evidence(&connection, base_kind, description, path);
        let sensitive = matches!(
            base_kind,
            EvidenceKind::SecretKey | EvidenceKind::ConfigMapKey
        );
        match lookup {
            Lookup::Resolved(target) => self.scan.add(LinkCandidate {
                source: source.clone(),
                target,
                via,
                kind: EdgeKind::ConnectsTo,
                confidence: connection.confidence,
                evidence,
            }),
            Lookup::Missing => {
                if connection.strong {
                    let unresolved = if sensitive {
                        UnresolvedTarget::new(REDACTED, None, REDACTED)
                    } else {
                        UnresolvedTarget::new(
                            connection.name.clone(),
                            connection.namespace.clone(),
                            connection.redacted.clone(),
                        )
                    };
                    self.scan.unresolved(source, unresolved, via);
                }
            }
            Lookup::External => {}
        }
    }
}

/// Structural facts about a scanned connection value.
struct ConnectionRef {
    name: String,
    namespace: Option<String>,
    redacted: String,
    confidence: f64,
    strong: bool,
    is_ip: bool,
    evidence_kind: EvidenceKind,
    class: Option<ports::PortClass>,
    scheme: Option<&'static str>,
}

fn classify_connection(raw: &str) -> Option<ConnectionRef> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.len() > MAX_VALUE_LEN {
        return None;
    }
    if let Some(uri) = uri::parse_uri_reference(trimmed) {
        let (name, namespace, is_ip) = match dns::parse_dns_reference(&uri.host) {
            Some(parsed) => (parsed.name, parsed.namespace, false),
            None if uri.host.parse::<IpAddr>().is_ok() => (uri.host.clone(), None, true),
            None => return None,
        };
        return Some(ConnectionRef {
            name,
            namespace,
            redacted: uri.redacted,
            confidence: if uri.namespace.is_some() { 0.9 } else { 0.85 },
            strong: true,
            is_ip,
            evidence_kind: EvidenceKind::Uri,
            class: Some(uri.class),
            scheme: Some(uri.class.scheme()),
        });
    }
    if let Some((address, port)) = dns::parse_ip_reference(trimmed) {
        let class = port.and_then(ports::classify_port);
        return Some(ConnectionRef {
            redacted: endpoint_preview(None, &address, port),
            name: address,
            namespace: None,
            confidence: if class.is_some() { 0.75 } else { 0.6 },
            strong: class.is_some(),
            is_ip: true,
            evidence_kind: EvidenceKind::DnsName,
            class,
            scheme: None,
        });
    }
    let parsed = dns::parse_dns_reference(trimmed)?;
    let class = parsed.port.and_then(ports::classify_port);
    let namespaced = parsed.namespace.is_some();
    let confidence = if parsed.cluster_scoped {
        0.9
    } else if namespaced && class.is_some() {
        0.85
    } else if namespaced {
        0.7
    } else if class.is_some() {
        0.75
    } else {
        0.55
    };
    let host = parsed.display_host();
    Some(ConnectionRef {
        redacted: endpoint_preview(None, &host, parsed.port),
        name: parsed.name,
        namespace: parsed.namespace,
        confidence,
        strong: parsed.cluster_scoped || namespaced || class.is_some(),
        is_ip: false,
        evidence_kind: EvidenceKind::DnsName,
        class,
        scheme: None,
    })
}

fn connection_evidence(
    connection: &ConnectionRef,
    base_kind: EvidenceKind,
    description: &str,
    path: &str,
) -> Vec<Evidence> {
    let field_path = || Some(knect_proto::FieldPath::new(path));
    let mut evidence = vec![
        Evidence {
            kind: base_kind,
            description: format!("{description} exposes {}", connection.redacted),
            path: field_path(),
            value_redacted: Some(connection.redacted.clone()),
        },
        Evidence {
            kind: connection.evidence_kind,
            description: connection_description(connection),
            path: field_path(),
            value_redacted: Some(connection.redacted.clone()),
        },
    ];
    if let Some(class) = connection.class {
        evidence.push(Evidence {
            kind: EvidenceKind::Port,
            description: format!(
                "port {} is the standard {} port",
                class.port(),
                class.label()
            ),
            path: field_path(),
            value_redacted: Some(connection.redacted.clone()),
        });
    }
    evidence
}

fn connection_description(connection: &ConnectionRef) -> String {
    if let Some(scheme) = connection.scheme {
        format!("{scheme} URI to {}", connection.redacted)
    } else if connection.is_ip {
        format!("endpoint address {}", connection.redacted)
    } else {
        format!("DNS name {}", connection.redacted)
    }
}

fn decode_secret_entry(entry: &str, raw: &str) -> Option<String> {
    if entry.starts_with("data/") {
        let bytes = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
        String::from_utf8(bytes).ok()
    } else if entry.starts_with("stringData/") {
        Some(raw.to_owned())
    } else {
        None
    }
}

fn reference_label(kind: &str, name: &str, key: Option<&str>) -> String {
    match key {
        Some(key) => format!("{kind} {name} key {key}"),
        None => format!("{kind} {name}"),
    }
}
