//! Deterministic topology diffing.
//!
//! A patch is computed from two projections: node identity is [`ResourceId`],
//! edge identity is the edge ID. Output vectors are sorted by identity so a
//! consumer applying the same patch twice observes the same order.
//!
//! The frozen protocol exposes `updated_nodes` but not `updated_edges`.
//! Changed edges therefore travel in `added_edges`, which consumers treat as an
//! upsert; the new weight replaces the previous edge with the same ID.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use knect_proto::ResourceId;
use knect_proto::events::GraphPatch;
use knect_proto::graph::{Edge, ResourceNode, Topology};

/// Diffs two topology projections.
///
/// `previous` is `None` for the first generation, which makes every node and
/// edge in `next` an addition.
#[must_use]
pub fn diff_topologies(
    previous: Option<&Topology>,
    next: &Topology,
    seq: u64,
    ts: DateTime<Utc>,
) -> GraphPatch {
    let previous_nodes = nodes_by_id(previous);
    let next_nodes = nodes_by_id(Some(next));
    let previous_edges = edges_by_id(previous);
    let next_edges = edges_by_id(Some(next));

    let mut added_nodes: Vec<ResourceNode> = next_nodes
        .iter()
        .filter(|(id, _)| !previous_nodes.contains_key(*id))
        .map(|(_, node)| (*node).clone())
        .collect();
    let mut updated_nodes: Vec<ResourceNode> = next_nodes
        .iter()
        .filter(|(id, node)| {
            previous_nodes
                .get(*id)
                .is_some_and(|previous| previous != *node)
        })
        .map(|(_, node)| (*node).clone())
        .collect();
    let mut removed_nodes: Vec<ResourceId> = previous_nodes
        .keys()
        .filter(|id| !next_nodes.contains_key(*id))
        .map(|id| (*id).clone())
        .collect();

    let mut added_edges: Vec<Edge> = next_edges
        .iter()
        .filter(|(id, edge)| {
            previous_edges
                .get(*id)
                .is_none_or(|previous| previous != *edge)
        })
        .map(|(_, edge)| (*edge).clone())
        .collect();
    let mut removed_edges: Vec<String> = previous_edges
        .keys()
        .filter(|id| !next_edges.contains_key(*id))
        .map(|id| (*id).to_owned())
        .collect();

    added_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    updated_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    removed_nodes.sort();
    added_edges.sort_by(|left, right| left.id.cmp(&right.id));
    removed_edges.sort();

    GraphPatch {
        generation: next.generation,
        added_nodes,
        updated_nodes,
        removed_nodes,
        added_edges,
        removed_edges,
        seq,
        ts,
    }
}

fn nodes_by_id(previous: Option<&Topology>) -> BTreeMap<&ResourceId, &ResourceNode> {
    previous
        .map(|topology| topology.nodes.iter().map(|node| (&node.id, node)).collect())
        .unwrap_or_default()
}

fn edges_by_id(previous: Option<&Topology>) -> BTreeMap<&str, &Edge> {
    previous
        .map(|topology| {
            topology
                .edges
                .iter()
                .map(|edge| (edge.id.as_str(), edge))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use knect_proto::graph::{EdgeKind, Health, TopologyStats};
    use knect_proto::{NodeCategory, SCHEMA_VERSION};

    use crate::graph::GraphBuilder;

    fn timestamp() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    fn node(name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", "Revision", "default", name),
            category: NodeCategory::Revision,
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: "1/1 replicas".to_owned(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: timestamp(),
            last_seen: timestamp(),
        }
    }

    fn edge(id: &str, from: &ResourceNode, to: &ResourceNode) -> Edge {
        Edge {
            id: id.to_owned(),
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn topology(generation: u64, nodes: Vec<ResourceNode>, edges: Vec<Edge>) -> Topology {
        let stats = TopologyStats {
            node_count: nodes.len() as u64,
            edge_count: edges.len() as u64,
            ..TopologyStats::default()
        };
        Topology {
            schema_version: SCHEMA_VERSION,
            context: "fixture".to_owned(),
            generation,
            nodes,
            edges,
            warnings: Vec::new(),
            stats,
        }
    }

    fn built_topology(generation: u64, names: &[&str]) -> Topology {
        let mut builder = GraphBuilder::new("fixture").with_generation(generation);
        for name in names {
            builder.add_node(node(name));
        }
        builder.build()
    }

    #[test]
    fn no_previous_projection_marks_everything_added() {
        let next = built_topology(1, &["b", "a"]);
        let patch = diff_topologies(None, &next, 9, timestamp());
        assert_eq!(patch.generation, 1);
        assert_eq!(patch.seq, 9);
        assert_eq!(patch.ts, timestamp());
        let names: Vec<&str> = patch
            .added_nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(patch.updated_nodes.is_empty());
        assert!(patch.removed_nodes.is_empty());
    }

    #[test]
    fn none_to_empty_is_an_empty_patch() {
        let next = topology(0, Vec::new(), Vec::new());
        let patch = diff_topologies(None, &next, 1, timestamp());
        assert!(patch.added_nodes.is_empty());
        assert!(patch.updated_nodes.is_empty());
        assert!(patch.removed_nodes.is_empty());
        assert!(patch.added_edges.is_empty());
        assert!(patch.removed_edges.is_empty());
    }

    #[test]
    fn added_updated_and_removed_nodes_are_classified() {
        let kept = node("kept");
        let removed = node("removed");
        let previous = topology(1, vec![kept.clone(), removed.clone()], Vec::new());

        let mut changed = kept.clone();
        changed.summary = "2/2 replicas".to_owned();
        let added = node("added");
        let next = topology(2, vec![changed.clone(), added.clone()], Vec::new());

        let patch = diff_topologies(Some(&previous), &next, 3, timestamp());
        assert_eq!(patch.generation, 2);
        assert_eq!(patch.added_nodes, vec![added]);
        assert_eq!(patch.updated_nodes, vec![changed]);
        assert_eq!(patch.removed_nodes, vec![removed.id]);
    }

    #[test]
    fn unchanged_topologies_produce_an_empty_patch() {
        let first = node("a");
        let previous = topology(1, vec![first.clone()], vec![edge("a-a", &first, &first)]);
        let patch = diff_topologies(Some(&previous), &previous, 4, timestamp());
        assert!(patch.added_nodes.is_empty());
        assert!(patch.updated_nodes.is_empty());
        assert!(patch.removed_nodes.is_empty());
        assert!(patch.added_edges.is_empty());
        assert!(patch.removed_edges.is_empty());
    }

    #[test]
    fn edge_additions_updates_and_removals_are_classified() {
        let a = node("a");
        let b = node("b");
        let kept = edge("kept", &a, &b);
        let removed = edge("removed", &a, &b);
        let previous = topology(
            1,
            vec![a.clone(), b.clone()],
            vec![kept.clone(), removed.clone()],
        );

        let mut changed = kept.clone();
        changed.label = Some("updated".to_owned());
        let added = edge("added", &b, &a);
        let next = topology(2, vec![a, b], vec![changed.clone(), added.clone()]);

        let patch = diff_topologies(Some(&previous), &next, 5, timestamp());
        assert_eq!(patch.added_edges, vec![added, changed]);
        assert_eq!(patch.removed_edges, vec!["removed".to_owned()]);
    }

    #[test]
    fn output_order_is_deterministic_regardless_of_input_order() {
        let a = node("a");
        let b = node("b");
        let previous = topology(1, Vec::new(), Vec::new());
        let next = topology(
            2,
            vec![b.clone(), a.clone()],
            vec![edge("z", &b, &a), edge("a", &a, &b)],
        );
        let patch = diff_topologies(Some(&previous), &next, 6, timestamp());
        let node_names: Vec<&str> = patch
            .added_nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        let edge_ids: Vec<&str> = patch
            .added_edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect();
        assert_eq!(node_names, vec!["a", "b"]);
        assert_eq!(edge_ids, vec!["a", "z"]);
    }
}
