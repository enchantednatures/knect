#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Fixture-driven serving correlation tests.

use std::path::Path;
use std::sync::Arc;

use knect_core::correlate::serving::ServingCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::store::ResourceStore;
use knect_proto::graph::{Edge, EdgeKind, Health, Topology};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Value, json};

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

fn load_store() -> ResourceStore {
    let contents = std::fs::read_to_string(fixture_path("serving/serving.json"))
        .expect("serving fixture is readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("serving fixture is valid JSON");
    let mut store = ResourceStore::new();
    for object in objects {
        store.upsert(object);
    }
    store
}

fn build_topology(store: &ResourceStore) -> Topology {
    let ctx = CorrelationContext::new("fixture", 1, Arc::new(store.clone()));
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    ServingCorrelator
        .correlate(&ctx, &mut graph)
        .expect("serving correlation succeeds");
    graph.build()
}

fn node_keys(topology: &Topology) -> Vec<(NodeCategory, String)> {
    let mut keys: Vec<(NodeCategory, String)> = topology
        .nodes
        .iter()
        .map(|node| (node.category, node.name.clone()))
        .collect();
    keys.sort();
    keys
}

fn edge_triples(topology: &Topology) -> Vec<(String, String, EdgeKind)> {
    let mut triples: Vec<(String, String, EdgeKind)> = topology
        .edges
        .iter()
        .map(|edge| (edge.from.to_string(), edge.to.to_string(), edge.kind))
        .collect();
    triples.sort();
    triples
}

fn serving_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("serving.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

fn autoscaling_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("autoscaling.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

fn apps_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("apps/v1", kind, "knect-e2e", name).to_string()
}

fn core_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("v1", kind, "knect-e2e", name).to_string()
}

fn triple(from: String, to: String, kind: EdgeKind) -> (String, String, EdgeKind) {
    (from, to, kind)
}

fn serving_triple(
    from: (&str, &str),
    to: (&str, &str),
    kind: EdgeKind,
) -> (String, String, EdgeKind) {
    triple(serving_id(from.0, from.1), serving_id(to.0, to.1), kind)
}

fn node<'a>(
    topology: &'a Topology,
    category: NodeCategory,
    name: &str,
) -> &'a knect_proto::ResourceNode {
    topology
        .nodes
        .iter()
        .find(|node| node.category == category && node.name == name)
        .unwrap_or_else(|| panic!("missing node {category:?}/{name}"))
}

fn badge_labels(node: &knect_proto::ResourceNode) -> Vec<&str> {
    node.badges
        .iter()
        .map(|badge| badge.label.as_str())
        .collect()
}

fn traffic_edge<'a>(topology: &'a Topology, revision: &str) -> &'a Edge {
    topology
        .edges
        .iter()
        .find(|edge| {
            edge.kind == EdgeKind::RoutesTo
                && edge.from.kind == "Route"
                && edge.from.name == "order-processor"
                && edge.to.name == revision
        })
        .unwrap_or_else(|| panic!("missing traffic edge to {revision}"))
}

#[test]
fn fixture_produces_exact_serving_nodes() {
    let topology = build_topology(&load_store());
    let expected = vec![
        (NodeCategory::KService, "inventory-service".to_owned()),
        (NodeCategory::KService, "order-processor".to_owned()),
        (NodeCategory::Route, "inventory-service".to_owned()),
        (NodeCategory::Route, "order-processor".to_owned()),
        (NodeCategory::Configuration, "inventory-service".to_owned()),
        (NodeCategory::Configuration, "order-processor".to_owned()),
        (NodeCategory::Revision, "inventory-service-00001".to_owned()),
        (NodeCategory::Revision, "order-processor-00001".to_owned()),
        (NodeCategory::Revision, "order-processor-00002".to_owned()),
        (NodeCategory::Revision, "order-processor-00003".to_owned()),
        (
            NodeCategory::PodAutoscaler,
            "inventory-service-00001".to_owned(),
        ),
        (
            NodeCategory::PodAutoscaler,
            "order-processor-00001".to_owned(),
        ),
        (
            NodeCategory::PodAutoscaler,
            "order-processor-00002".to_owned(),
        ),
        (
            NodeCategory::PodAutoscaler,
            "order-processor-00003".to_owned(),
        ),
        (
            NodeCategory::Deployment,
            "inventory-service-00001-deployment".to_owned(),
        ),
        (
            NodeCategory::Deployment,
            "order-processor-00001-deployment".to_owned(),
        ),
        (
            NodeCategory::Deployment,
            "order-processor-00002-deployment".to_owned(),
        ),
        (
            NodeCategory::Deployment,
            "order-processor-00003-deployment".to_owned(),
        ),
        (NodeCategory::Pod, "inventory-service-00001-ghi".to_owned()),
        (NodeCategory::Pod, "order-processor-00001-abc".to_owned()),
        (NodeCategory::Pod, "order-processor-00003-def".to_owned()),
    ];
    assert_eq!(node_keys(&topology), expected);
}

#[test]
fn fixture_produces_exact_serving_edges() {
    let topology = build_topology(&load_store());
    let mut expected = vec![
        serving_triple(
            ("Service", "order-processor"),
            ("Route", "order-processor"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Service", "order-processor"),
            ("Configuration", "order-processor"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Service", "inventory-service"),
            ("Route", "inventory-service"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Service", "inventory-service"),
            ("Configuration", "inventory-service"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Configuration", "order-processor"),
            ("Revision", "order-processor-00001"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Configuration", "order-processor"),
            ("Revision", "order-processor-00002"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Configuration", "order-processor"),
            ("Revision", "order-processor-00003"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Configuration", "inventory-service"),
            ("Revision", "inventory-service-00001"),
            EdgeKind::Owns,
        ),
        serving_triple(
            ("Route", "order-processor"),
            ("Revision", "order-processor-00001"),
            EdgeKind::RoutesTo,
        ),
        serving_triple(
            ("Route", "order-processor"),
            ("Revision", "order-processor-00002"),
            EdgeKind::RoutesTo,
        ),
        serving_triple(
            ("Route", "inventory-service"),
            ("Revision", "inventory-service-00001"),
            EdgeKind::RoutesTo,
        ),
        triple(
            serving_id("Revision", "order-processor-00001"),
            autoscaling_id("PodAutoscaler", "order-processor-00001"),
            EdgeKind::Scales,
        ),
        triple(
            serving_id("Revision", "order-processor-00002"),
            autoscaling_id("PodAutoscaler", "order-processor-00002"),
            EdgeKind::Scales,
        ),
        triple(
            serving_id("Revision", "order-processor-00003"),
            autoscaling_id("PodAutoscaler", "order-processor-00003"),
            EdgeKind::Scales,
        ),
        triple(
            serving_id("Revision", "inventory-service-00001"),
            autoscaling_id("PodAutoscaler", "inventory-service-00001"),
            EdgeKind::Scales,
        ),
        triple(
            autoscaling_id("PodAutoscaler", "order-processor-00001"),
            apps_id("Deployment", "order-processor-00001-deployment"),
            EdgeKind::Scales,
        ),
        triple(
            autoscaling_id("PodAutoscaler", "order-processor-00002"),
            apps_id("Deployment", "order-processor-00002-deployment"),
            EdgeKind::Scales,
        ),
        triple(
            autoscaling_id("PodAutoscaler", "order-processor-00003"),
            apps_id("Deployment", "order-processor-00003-deployment"),
            EdgeKind::Scales,
        ),
        triple(
            autoscaling_id("PodAutoscaler", "inventory-service-00001"),
            apps_id("Deployment", "inventory-service-00001-deployment"),
            EdgeKind::Scales,
        ),
        triple(
            apps_id("Deployment", "order-processor-00001-deployment"),
            core_id("Pod", "order-processor-00001-abc"),
            EdgeKind::Runs,
        ),
        triple(
            apps_id("Deployment", "order-processor-00003-deployment"),
            core_id("Pod", "order-processor-00003-def"),
            EdgeKind::Runs,
        ),
        triple(
            apps_id("Deployment", "inventory-service-00001-deployment"),
            core_id("Pod", "inventory-service-00001-ghi"),
            EdgeKind::Runs,
        ),
    ];
    expected.sort();
    assert_eq!(edge_triples(&topology), expected);
}

#[test]
fn route_traffic_metadata_carries_percent_tag_and_url() {
    let topology = build_topology(&load_store());
    let stable = traffic_edge(&topology, "order-processor-00001");
    assert_eq!(stable.label.as_deref(), Some("90% stable"));
    assert_eq!(stable.metadata.get("percent"), Some(&json!(90)));
    assert_eq!(stable.metadata.get("tag"), Some(&json!("stable")));
    assert_eq!(stable.metadata.get("latest_revision"), Some(&json!(false)));
    assert_eq!(
        stable.metadata.get("url").and_then(Value::as_str),
        Some("http://stable-order-processor.knect-e2e.sslip.io")
    );

    let canary = traffic_edge(&topology, "order-processor-00002");
    assert_eq!(canary.label.as_deref(), Some("10% canary"));
    assert_eq!(canary.metadata.get("percent"), Some(&json!(10)));
    assert_eq!(canary.metadata.get("tag"), Some(&json!("canary")));

    let latest = topology
        .edges
        .iter()
        .find(|edge| {
            edge.kind == EdgeKind::RoutesTo
                && edge.from.name == "inventory-service"
                && edge.to.name == "inventory-service-00001"
        })
        .expect("latest revision route edge");
    assert_eq!(latest.metadata.get("percent"), Some(&json!(100)));
    assert_eq!(latest.metadata.get("latest_revision"), Some(&json!(true)));
}

#[test]
fn revisions_expose_all_four_scale_states() {
    let topology = build_topology(&load_store());

    let active = node(&topology, NodeCategory::Revision, "order-processor-00001");
    assert_eq!(active.health, Health::Healthy);
    assert!(badge_labels(active).contains(&"active"));
    assert_eq!(active.summary, "1/1 replicas");

    let zero = node(&topology, NodeCategory::Revision, "order-processor-00002");
    assert_eq!(zero.health, Health::Healthy);
    assert!(badge_labels(zero).contains(&"scaled-to-zero"));
    assert_eq!(zero.summary, "0/0 replicas");
    assert_eq!(
        zero.badges
            .iter()
            .find(|badge| badge.label == "scaled-to-zero")
            .and_then(|badge| badge.tooltip.as_deref()),
        Some("0K")
    );

    let scaling = node(&topology, NodeCategory::Revision, "inventory-service-00001");
    assert_eq!(scaling.health, Health::Progressing);
    assert!(badge_labels(scaling).contains(&"scaling-up"));
    assert!(scaling.summary.contains("0/1 replicas"));

    let degraded = node(&topology, NodeCategory::Revision, "order-processor-00003");
    assert_eq!(degraded.health, Health::Degraded);
    assert!(badge_labels(degraded).contains(&"degraded"));
    assert!(badge_labels(degraded).contains(&"crashloop"));
    assert!(degraded.summary.contains("0/1 replicas"));
    assert!(degraded.summary.contains("ProgressDeadlineExceeded"));
}

#[test]
fn removing_a_revision_removes_its_route_edge() {
    let mut store = load_store();
    let revision_id = ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Revision",
        "knect-e2e",
        "order-processor-00002",
    );
    store.remove(&revision_id);

    let topology = build_topology(&store);
    assert!(!topology.nodes.iter().any(|node| node.id == revision_id));
    let route_edges: Vec<&Edge> = topology
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.from.name == "order-processor")
        .collect();
    assert_eq!(route_edges.len(), 1);
    assert_eq!(route_edges[0].to.name, "order-processor-00001");
    assert!(
        topology
            .warnings
            .iter()
            .any(|warning| warning.code == "missing_revision")
    );
}

#[test]
fn repeated_runs_are_deterministic() {
    let store = load_store();
    assert_eq!(build_topology(&store), build_topology(&store));
}
