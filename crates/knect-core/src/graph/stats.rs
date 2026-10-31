//! Deterministic topology statistics.
//!
//! Category counts are keyed by the protocol's frozen snake_case category name
//! so repeated projections produce byte-for-byte identical statistics.

use std::collections::BTreeMap;

use knect_proto::NodeCategory;
use knect_proto::graph::{CategoryCount, Edge, ResourceNode, TopologyStats, Warning};

/// Computes topology statistics with `truncated` set to `false`.
#[must_use]
pub fn stats_for(nodes: &[ResourceNode], edges: &[Edge], warnings: &[Warning]) -> TopologyStats {
    stats_for_truncated(nodes, edges, warnings, false)
}

/// Computes topology statistics, preserving the caller's truncation flag.
#[must_use]
pub fn stats_for_truncated(
    nodes: &[ResourceNode],
    edges: &[Edge],
    warnings: &[Warning],
    truncated: bool,
) -> TopologyStats {
    let mut categories: BTreeMap<&'static str, (NodeCategory, u64)> = BTreeMap::new();
    for node in nodes {
        let entry = categories
            .entry(category_key(node.category))
            .or_insert((node.category, 0));
        entry.1 = entry.1.saturating_add(1);
    }
    let categories: Vec<CategoryCount> = categories
        .into_values()
        .map(|(category, count)| CategoryCount { category, count })
        .collect();
    TopologyStats {
        node_count: nodes.len() as u64,
        edge_count: edges.len() as u64,
        warning_count: warnings.len() as u64,
        categories,
        truncated,
    }
}

/// Stable sort key matching the protocol's snake_case category name.
pub(crate) const fn category_key(category: NodeCategory) -> &'static str {
    match category {
        NodeCategory::KService => "k_service",
        NodeCategory::Route => "route",
        NodeCategory::Configuration => "configuration",
        NodeCategory::Revision => "revision",
        NodeCategory::PodAutoscaler => "pod_autoscaler",
        NodeCategory::Broker => "broker",
        NodeCategory::Trigger => "trigger",
        NodeCategory::Source => "source",
        NodeCategory::SinkBinding => "sink_binding",
        NodeCategory::Subscription => "subscription",
        NodeCategory::Channel => "channel",
        NodeCategory::Deployment => "deployment",
        NodeCategory::ReplicaSet => "replica_set",
        NodeCategory::Pod => "pod",
        NodeCategory::Service => "service",
        NodeCategory::Endpoints => "endpoints",
        NodeCategory::BackingService => "backing_service",
        NodeCategory::Topic => "topic",
        NodeCategory::StorageClass => "storage_class",
        NodeCategory::PersistentVolume => "persistent_volume",
        NodeCategory::PersistentVolumeClaim => "persistent_volume_claim",
        NodeCategory::CephPool => "ceph_pool",
        NodeCategory::Policy => "policy",
        NodeCategory::Canary => "canary",
        NodeCategory::VirtualService => "virtual_service",
        NodeCategory::DestinationRule => "destination_rule",
        NodeCategory::PeerAuthentication => "peer_authentication",
        NodeCategory::AuthorizationPolicy => "authorization_policy",
        NodeCategory::ConfigMap => "config_map",
        NodeCategory::Secret => "secret",
        NodeCategory::Namespace => "namespace",
        NodeCategory::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use knect_proto::ResourceId;
    use knect_proto::graph::Health;

    fn node(category: NodeCategory, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("v1", "Pod", "default", name),
            category,
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: timestamp(),
            last_seen: timestamp(),
        }
    }

    fn timestamp() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    fn warning(code: &str) -> Warning {
        Warning {
            code: code.to_owned(),
            message: code.to_owned(),
            severity: knect_proto::graph::Severity::Info,
            resource: None,
        }
    }

    #[test]
    fn empty_topology_has_zero_stats() {
        assert_eq!(stats_for(&[], &[], &[]), TopologyStats::default());
    }

    #[test]
    fn category_counts_are_sorted_and_aggregated() {
        let nodes = vec![
            node(NodeCategory::Pod, "web"),
            node(NodeCategory::KService, "api"),
            node(NodeCategory::Pod, "worker"),
            node(NodeCategory::Route, "api"),
        ];
        let stats = stats_for(&nodes, &[], &[warning("a"), warning("b")]);
        assert_eq!(stats.node_count, 4);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.warning_count, 2);
        assert!(!stats.truncated);
        let categories: Vec<(&str, u64)> = stats
            .categories
            .iter()
            .map(|count| (category_key(count.category), count.count))
            .collect();
        assert_eq!(categories, vec![("k_service", 1), ("pod", 2), ("route", 1)]);
    }

    #[test]
    fn truncation_flag_is_preserved() {
        let stats = stats_for_truncated(&[], &[], &[], true);
        assert!(stats.truncated);
    }

    #[test]
    fn repeated_computation_is_identical() {
        let nodes = vec![node(NodeCategory::Pod, "web")];
        assert_eq!(stats_for(&nodes, &[], &[]), stats_for(&nodes, &[], &[]));
    }
}
