//! Resource parsing and daemon access shared by one-shot CLI commands.

use std::path::Path;
use std::sync::Arc;

use knect_proto::graph::NodeCategory;
use knect_proto::rpc::{GetTopologyParams, methods};
use knect_proto::{ResourceId, ResourceNode, Topology};

use super::CliError;
use super::kind;
pub use super::kind::KindSpec;
use crate::client::{self, ClientError, ConnectOptions, Connected, RpcClient, resolve_endpoint};

/// Parsed `<kind>/<name>` or `<apiVersion>/<kind>/<namespace>/<name>` target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTarget {
    /// Original argument.
    pub raw: String,
    /// Kind constraint, when supplied.
    pub kind: Option<KindSpec>,
    /// Object name.
    pub name: String,
    /// Namespace embedded in a four-segment identifier.
    pub namespace: Option<String>,
}

/// Parses a resource argument.
pub fn parse_target(spec: &str) -> Result<ParsedTarget, CliError> {
    let parts: Vec<&str> = spec.split('/').collect();
    let target = match parts.as_slice() {
        [name] if !name.is_empty() => ParsedTarget {
            raw: spec.to_owned(),
            kind: None,
            name: (*name).to_owned(),
            namespace: None,
        },
        [kind, name] if !kind.is_empty() && !name.is_empty() => ParsedTarget {
            raw: spec.to_owned(),
            kind: Some(kind::kind_spec(kind)),
            name: (*name).to_owned(),
            namespace: None,
        },
        [group, version, kind, namespace, name]
            if !group.is_empty()
                && !version.is_empty()
                && !kind.is_empty()
                && !namespace.is_empty()
                && !name.is_empty() =>
        {
            ParsedTarget {
                raw: spec.to_owned(),
                kind: Some(KindSpec {
                    kind: kind::canonical_kind(kind),
                    api_version: Some(format!("{group}/{version}")),
                    category: None,
                }),
                name: (*name).to_owned(),
                namespace: Some((*namespace).to_owned()),
            }
        }
        [api_version, kind, namespace, name]
            if !api_version.is_empty()
                && !kind.is_empty()
                && !namespace.is_empty()
                && !name.is_empty() =>
        {
            ParsedTarget {
                raw: spec.to_owned(),
                kind: Some(KindSpec {
                    kind: kind::canonical_kind(kind),
                    api_version: Some((*api_version).to_owned()),
                    category: None,
                }),
                name: (*name).to_owned(),
                namespace: Some((*namespace).to_owned()),
            }
        }
        _ => {
            return Err(CliError::Message(format!(
                "invalid resource '{spec}': use <kind>/<name> or <apiVersion>/<kind>/<namespace>/<name>"
            )));
        }
    };
    Ok(target)
}

/// Resolves a parsed target against a topology.
pub fn resolve_target(
    topology: &Topology,
    target: &ParsedTarget,
    namespace: Option<&str>,
) -> Result<ResourceId, CliError> {
    let namespace = namespace.or(target.namespace.as_deref());
    let matches: Vec<&ResourceNode> = topology
        .nodes
        .iter()
        .filter(|node| node.name == target.name)
        .filter(|node| namespace.is_none_or(|value| node.namespace.as_deref() == Some(value)))
        .filter(|node| {
            target
                .kind
                .as_ref()
                .is_none_or(|kind| node_matches(node, kind))
        })
        .collect();
    match matches.as_slice() {
        [] => Err(CliError::NotFound(format!(
            "{} in namespace {}",
            target.raw,
            namespace.unwrap_or("any")
        ))),
        [node] => Ok(node.id.clone()),
        many => Err(CliError::Ambiguous(format!(
            "{} matches {}; qualify the kind or namespace",
            target.raw,
            many.iter()
                .map(|node| node.id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// True when a topology node satisfies a kind constraint.
#[must_use]
pub fn node_matches(node: &ResourceNode, kind: &KindSpec) -> bool {
    if let Some(api_version) = kind.api_version.as_deref()
        && node.id.api_version != api_version
    {
        return false;
    }
    if node.id.kind.eq_ignore_ascii_case(&kind.kind) {
        return true;
    }
    kind.category
        .is_some_and(|category| node.category == category)
}

/// Connects to the daemon and keeps the spawn handle alive.
pub async fn connect(
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<Connected, CliError> {
    let explicit = socket.is_some() || daemon_url.is_some();
    let endpoint = resolve_endpoint(socket, daemon_url)?;
    let connected = client::connect(ConnectOptions::new(endpoint, explicit)).await?;
    if !connected.reachable {
        let message = connected
            .warning
            .clone()
            .unwrap_or_else(|| "daemon is not reachable".to_owned());
        return Err(CliError::Client(ClientError::Transport(message)));
    }
    Ok(connected)
}

/// Fetches the current topology.
pub async fn fetch_topology(client: Arc<dyn RpcClient>) -> Result<Topology, CliError> {
    let params = serde_json::to_value(GetTopologyParams::default())
        .map_err(|error| CliError::Message(format!("cannot encode topology params: {error}")))?;
    let value = client
        .call(methods::GRAPH_GET_TOPOLOGY.to_owned(), Some(params))
        .await?;
    serde_json::from_value(value).map_err(|error| {
        CliError::Client(ClientError::Protocol(format!(
            "cannot decode topology: {error}"
        )))
    })
}

/// Human label for a node category.
#[must_use]
pub fn category_label(category: NodeCategory) -> &'static str {
    match category {
        NodeCategory::KService => "KService",
        NodeCategory::Route => "Route",
        NodeCategory::Configuration => "Configuration",
        NodeCategory::Revision => "Revision",
        NodeCategory::PodAutoscaler => "PodAutoscaler",
        NodeCategory::Broker => "Broker",
        NodeCategory::Trigger => "Trigger",
        NodeCategory::EventType => "EventType",
        NodeCategory::Source => "Source",
        NodeCategory::SinkBinding => "SinkBinding",
        NodeCategory::Subscription => "Subscription",
        NodeCategory::Channel => "Channel",
        NodeCategory::Sink => "Sink",
        NodeCategory::Deployment => "Deployment",
        NodeCategory::ReplicaSet => "ReplicaSet",
        NodeCategory::Pod => "Pod",
        NodeCategory::Service => "Service",
        NodeCategory::Endpoints => "Endpoints",
        NodeCategory::BackingService => "BackingService",
        NodeCategory::Topic => "Topic",
        NodeCategory::StorageClass => "StorageClass",
        NodeCategory::PersistentVolume => "PersistentVolume",
        NodeCategory::PersistentVolumeClaim => "PersistentVolumeClaim",
        NodeCategory::CephPool => "CephPool",
        NodeCategory::Policy => "Policy",
        NodeCategory::Canary => "Canary",
        NodeCategory::VirtualService => "VirtualService",
        NodeCategory::DestinationRule => "DestinationRule",
        NodeCategory::PeerAuthentication => "PeerAuthentication",
        NodeCategory::AuthorizationPolicy => "AuthorizationPolicy",
        NodeCategory::ConfigMap => "ConfigMap",
        NodeCategory::Secret => "Secret",
        NodeCategory::Namespace => "Namespace",
        NodeCategory::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::graph::{Health, TopologyStats};
    use std::collections::BTreeMap;

    fn node(category: NodeCategory, kind: &str, namespace: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", kind, namespace, name),
            category,
            name: name.to_owned(),
            namespace: Some(namespace.to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn topology() -> Topology {
        Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            nodes: vec![
                node(NodeCategory::KService, "Service", "default", "hello"),
                node(NodeCategory::Broker, "Broker", "default", "orders"),
            ],
            edges: Vec::new(),
            warnings: Vec::new(),
            stats: TopologyStats::default(),
        }
    }

    #[test]
    fn parses_kind_and_four_segment_forms() {
        let target = parse_target("ksvc/hello").expect("parse");
        assert_eq!(target.name, "hello");
        assert_eq!(
            target.kind.expect("kind").category,
            Some(NodeCategory::KService)
        );
        let target = parse_target("serving.knative.dev/v1/Service/default/hello").expect("parse");
        assert_eq!(target.namespace.as_deref(), Some("default"));
        assert!(parse_target("a/b/c").is_err());
        assert!(parse_target("").is_err());
    }

    #[test]
    fn resolves_aliases_and_reports_ambiguity() {
        let topology = topology();
        let target = parse_target("ksvc/hello").expect("parse");
        assert_eq!(
            resolve_target(&topology, &target, Some("default"))
                .expect("resolve")
                .kind,
            "Service"
        );
        let target = parse_target("hero").expect("bare");
        assert!(matches!(
            resolve_target(&topology, &target, None),
            Err(CliError::NotFound(_))
        ));
        let target = parse_target("hello").expect("bare");
        assert!(resolve_target(&topology, &target, None).is_ok());
    }
}
