//! `graph.*` RPC handlers.

use std::collections::BTreeMap;
use std::time::Duration;

use knect_proto::error::{INTERNAL_ERROR, NOT_FOUND};
use knect_proto::graph::{Edge, EdgeKind, NodeCategory, NodeDetails, Severity, Topology};
use knect_proto::links::{LinkRecord, LinkVia};
use knect_proto::policy::{PolicyKind, PolicyRecord, PolicyResult};
use knect_proto::rpc::{GetNodeDetailsParams, GetTopologyParams, TopologySection};
use knect_proto::storage::{AccessMode, StorageConflictWarning, StorageRecord, StorageWarning};
use knect_proto::{ResourceId, RpcError};

use crate::app::AppState;

/// Maximum time `min_generation` may wait for catch-up.
const MIN_GENERATION_WAIT: Duration = Duration::from_secs(2);
/// Poll cadence for the bounded generation wait.
const GENERATION_POLL: Duration = Duration::from_millis(25);

/// Returns the current topology, optionally waiting briefly for a generation.
pub(crate) async fn get_topology(
    state: &AppState,
    params: &GetTopologyParams,
) -> Result<Topology, RpcError> {
    if let Some(context) = params.context.as_deref()
        && !context.is_empty()
        && context != state.context()
    {
        return Err(RpcError::new(
            NOT_FOUND,
            format!("context '{context}' is not served by this daemon"),
        ));
    }

    if let Some(min_generation) = params.min_generation {
        let deadline = tokio::time::Instant::now() + MIN_GENERATION_WAIT;
        while state.generation() < min_generation && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(GENERATION_POLL).await;
        }
    }

    let mut topology = (*state.topology()).clone();
    if let Some(include) = params.include.as_ref() {
        if !include.contains(&TopologySection::Nodes) {
            topology.nodes.clear();
        }
        if !include.contains(&TopologySection::Edges) {
            topology.edges.clear();
        }
        if !include.contains(&TopologySection::Warnings) {
            topology.warnings.clear();
        }
        if !include.contains(&TopologySection::Stats) {
            topology.stats = knect_proto::graph::TopologyStats::default();
        }
    }
    Ok(topology)
}

/// Builds node details from the topology projection and the sanitized store.
pub(crate) async fn get_node_details(
    state: &AppState,
    params: &GetNodeDetailsParams,
) -> Result<NodeDetails, RpcError> {
    let topology = state.topology();
    let node = topology
        .nodes
        .iter()
        .find(|node| node.id == params.id)
        .cloned()
        .ok_or_else(|| {
            RpcError::new(
                NOT_FOUND,
                format!("node {} is not present in the topology", params.id),
            )
        })?;

    let object = {
        let store = state.store.read().await;
        store.get(&params.id).map(|object| (*object).clone())
    };

    let yaml = if params.include_yaml.unwrap_or(false) {
        match object.as_ref() {
            Some(object) => Some(serde_yaml::to_string(object).map_err(|error| {
                RpcError::new(
                    INTERNAL_ERROR,
                    format!("cannot serialize node YAML: {error}"),
                )
            })?),
            None => None,
        }
    } else {
        None
    };

    let warnings = topology
        .warnings
        .iter()
        .filter(|warning| warning.resource.as_ref() == Some(&params.id))
        .cloned()
        .collect();
    let links = collect_links(&topology, &params.id);
    let policies = collect_policies(&topology, &params.id);
    let storage = collect_storage(&topology, &params.id);
    let mutations = object
        .as_ref()
        .map(|object| knect_core::mutations::records(object).unwrap_or_default())
        .unwrap_or_default();

    Ok(NodeDetails {
        id: node.id,
        category: node.category,
        name: node.name,
        namespace: node.namespace,
        health: node.health,
        summary: node.summary,
        yaml,
        yaml_redacted: true,
        mutations,
        policies,
        links,
        storage,
        warnings,
        metrics: None,
        logs: Vec::new(),
    })
}

fn incident_edges<'a>(topology: &'a Topology, id: &ResourceId) -> impl Iterator<Item = &'a Edge> {
    topology
        .edges
        .iter()
        .filter(move |edge| &edge.from == id || &edge.to == id)
}

fn metadata_str<'a>(edge: &'a Edge, key: &str) -> Option<&'a str> {
    edge.metadata.get(key).and_then(serde_json::Value::as_str)
}

fn collect_links(topology: &Topology, id: &ResourceId) -> Vec<LinkRecord> {
    let mut links = BTreeMap::new();
    for edge in incident_edges(topology, id) {
        if let Some(value) = edge.metadata.get("link")
            && let Ok(record) = serde_json::from_value::<LinkRecord>(value.clone())
        {
            links.insert(edge.id.clone(), record);
            continue;
        }
        if let Some(record) = synthesized_link(edge) {
            links.insert(edge.id.clone(), record);
        }
    }
    links.into_values().collect()
}

fn synthesized_link(edge: &Edge) -> Option<LinkRecord> {
    let via = match edge.kind {
        EdgeKind::UsesSecret => LinkVia::Secret,
        EdgeKind::UsesConfigMap => LinkVia::ConfigMap,
        EdgeKind::BackedBy | EdgeKind::ConsumesFrom | EdgeKind::Selects => LinkVia::OperatorCrd,
        EdgeKind::Mounts | EdgeKind::ProvisionedBy => LinkVia::Storage,
        EdgeKind::ConnectsTo | EdgeKind::References => LinkVia::Dns,
        EdgeKind::DeliversTo | EdgeKind::Subscribes | EdgeKind::DeadLetterSink => {
            LinkVia::ExplicitBinding
        }
        _ => return None,
    };
    Some(LinkRecord {
        source: edge.from.clone(),
        target: edge.to.clone(),
        via,
        confidence: 1.0,
        evidence: Vec::new(),
        health: edge.health,
    })
}

fn collect_policies(topology: &Topology, id: &ResourceId) -> Vec<PolicyRecord> {
    let mut policies = BTreeMap::new();
    for edge in incident_edges(topology, id).filter(|edge| {
        matches!(
            edge.kind,
            EdgeKind::ProtectedBy | EdgeKind::MutatedBy | EdgeKind::Targets | EdgeKind::Manages
        )
    }) {
        let other = if &edge.from == id {
            &edge.to
        } else {
            &edge.from
        };
        let node = topology.nodes.iter().find(|node| &node.id == other);
        let name = node
            .map(|node| node.name.clone())
            .unwrap_or_else(|| other.name.clone());
        let namespace = node
            .and_then(|node| node.namespace.clone())
            .or_else(|| other.namespace.clone());
        let severity = parse_severity(metadata_str(edge, "severity"))
            .or_else(|| edge.warnings.first().map(|warning| warning.severity))
            .unwrap_or(Severity::Info);
        let message = metadata_str(edge, "message")
            .map(ToOwned::to_owned)
            .or_else(|| node.map(|node| node.summary.clone()))
            .unwrap_or_default();
        let mut details = BTreeMap::new();
        for key in [
            "rule",
            "event",
            "report",
            "policy",
            "managed_by",
            "kind",
            "reason",
        ] {
            if let Some(value) = metadata_str(edge, key) {
                details.insert(key.to_owned(), value.to_owned());
            }
        }
        policies.insert(
            edge.id.clone(),
            PolicyRecord {
                id: edge.id.clone(),
                name,
                namespace,
                kind: policy_kind(other),
                resource: Some(id.clone()),
                severity,
                result: parse_policy_result(metadata_str(edge, "result")),
                message,
                observed_at: None,
                details,
            },
        );
    }
    policies.into_values().collect()
}

fn policy_kind(id: &ResourceId) -> PolicyKind {
    let api = id.api_version.to_ascii_lowercase();
    if api.contains("kyverno") {
        PolicyKind::Kyverno
    } else if api.contains("istio") {
        PolicyKind::Istio
    } else if api.contains("flagger") {
        PolicyKind::Flagger
    } else {
        PolicyKind::Other
    }
}

fn parse_policy_result(value: Option<&str>) -> PolicyResult {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("fail") => PolicyResult::Fail,
        Some("warn" | "warning") => PolicyResult::Warn,
        Some("error") => PolicyResult::Error,
        Some("skip") => PolicyResult::Skip,
        _ => PolicyResult::Pass,
    }
}

fn parse_severity(value: Option<&str>) -> Option<Severity> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "critical" => Some(Severity::Critical),
        "high" => Some(Severity::High),
        "medium" => Some(Severity::Medium),
        "low" => Some(Severity::Low),
        "info" | "informational" => Some(Severity::Info),
        _ => None,
    }
}

fn collect_storage(topology: &Topology, id: &ResourceId) -> Vec<StorageRecord> {
    let mut storage = BTreeMap::new();
    for edge in incident_edges(topology, id).filter(|edge| {
        matches!(
            edge.kind,
            EdgeKind::Mounts | EdgeKind::ProvisionedBy | EdgeKind::BackedBy
        )
    }) {
        let from_node = topology.nodes.iter().find(|node| node.id == edge.from);
        let to_node = topology.nodes.iter().find(|node| node.id == edge.to);
        let from_is_pvc =
            from_node.is_some_and(|node| node.category == NodeCategory::PersistentVolumeClaim);
        let to_is_pvc =
            to_node.is_some_and(|node| node.category == NodeCategory::PersistentVolumeClaim);
        let (workload, pvc) = match edge.kind {
            EdgeKind::Mounts => {
                if from_is_pvc {
                    (Some(edge.to.clone()), edge.from.clone())
                } else if to_is_pvc {
                    (Some(edge.from.clone()), edge.to.clone())
                } else {
                    continue;
                }
            }
            EdgeKind::ProvisionedBy | EdgeKind::BackedBy => {
                if from_is_pvc {
                    (None, edge.from.clone())
                } else if to_is_pvc {
                    (None, edge.to.clone())
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        let access_modes = edge
            .metadata
            .get("access_modes")
            .and_then(serde_json::Value::as_array)
            .map(|modes| {
                modes
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(parse_access_mode)
                    .collect()
            })
            .unwrap_or_default();
        let warnings = edge
            .warnings
            .iter()
            .map(|warning| StorageWarning {
                code: warning.code.clone(),
                message: warning.message.clone(),
                severity: warning.severity,
                conflict: storage_conflict(edge, &pvc),
            })
            .collect();
        storage.insert(
            format!("{}|{}", edge.id, pvc),
            StorageRecord {
                pvc: pvc.clone(),
                volume: metadata_str(edge, "volume").map(ToOwned::to_owned),
                storage_class: metadata_str(edge, "storage_class")
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        (edge.kind == EdgeKind::ProvisionedBy).then(|| edge.to.name.clone())
                    }),
                access_modes,
                capacity: None,
                phase: None,
                mounted_by: workload.into_iter().collect(),
                provisioned_by: (edge.kind == EdgeKind::ProvisionedBy).then(|| edge.to.clone()),
                backend: (edge.kind == EdgeKind::BackedBy).then(|| edge.to.clone()),
                warnings,
            },
        );
    }
    storage.into_values().collect()
}

fn storage_conflict(edge: &Edge, pvc: &ResourceId) -> Option<StorageConflictWarning> {
    let access_mode = parse_access_mode(metadata_str(edge, "conflict_access_mode")?);
    let max_scale = edge
        .metadata
        .get("conflict_max_scale")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    Some(StorageConflictWarning {
        pvc: pvc.clone(),
        access_mode,
        max_scale,
        reason: "ReadWriteOnce claim cannot satisfy concurrent autoscaled consumers".to_owned(),
        affected: vec![edge.from.clone()],
    })
}

fn parse_access_mode(value: &str) -> AccessMode {
    match value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_', ' '], "")
        .as_str()
    {
        "readwriteonce" => AccessMode::ReadWriteOnce,
        "readwriteoncepod" => AccessMode::ReadWriteOncePod,
        "readwritemany" => AccessMode::ReadWriteMany,
        "readonlymany" => AccessMode::ReadOnlyMany,
        _ => AccessMode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::graph::Health;

    fn eventing_edge(kind: EdgeKind) -> Edge {
        Edge {
            id: format!("{kind:?}"),
            from: ResourceId::namespaced(
                "eventing.knative.dev/v1",
                "Trigger",
                "marrakesh",
                "marrakesh-events-trigger",
            ),
            to: ResourceId::namespaced(
                "serving.knative.dev/v1",
                "Service",
                "marrakesh",
                "marrakesh",
            ),
            kind,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn eventing_edges_surface_as_explicit_links() {
        for kind in [
            EdgeKind::DeliversTo,
            EdgeKind::Subscribes,
            EdgeKind::DeadLetterSink,
        ] {
            let link = synthesized_link(&eventing_edge(kind)).expect("eventing link");
            assert_eq!(link.via, LinkVia::ExplicitBinding);
        }
    }

    #[test]
    fn eventing_edges_surface_in_incident_node_links() {
        let service = ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            "marrakesh",
            "marrakesh",
        );
        let mut topology = crate::app::empty_topology("test", 1);
        topology.edges.push(eventing_edge(EdgeKind::DeliversTo));

        let links = collect_links(&topology, &service);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].via, LinkVia::ExplicitBinding);
        assert_eq!(links[0].source.kind, "Trigger");
        assert_eq!(links[0].target, service);
    }
}
