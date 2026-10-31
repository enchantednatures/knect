#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Fixture-driven eventing correlation tests.

use std::path::Path;
use std::sync::Arc;

use knect_core::correlate::eventing::EventingCorrelator;
use knect_core::correlate::serving::ServingCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::store::ResourceStore;
use knect_proto::graph::{Edge, EdgeKind, Topology};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::Value;

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

fn load_store() -> ResourceStore {
    let contents = std::fs::read_to_string(fixture_path("eventing/eventing.json"))
        .expect("eventing fixture is readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("eventing fixture is valid JSON");
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
    EventingCorrelator
        .correlate(&ctx, &mut graph)
        .expect("eventing correlation succeeds");
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

fn eventing_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("eventing.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

fn messaging_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("messaging.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

fn source_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("sources.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

fn apps_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("apps/v1", kind, "knect-e2e", name).to_string()
}

fn triple(from: String, to: String, kind: EdgeKind) -> (String, String, EdgeKind) {
    (from, to, kind)
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

fn edge<'a>(topology: &'a Topology, from: &str, to: &str, kind: EdgeKind) -> &'a Edge {
    topology
        .edges
        .iter()
        .find(|edge| edge.kind == kind && edge.from.name == from && edge.to.name == to)
        .unwrap_or_else(|| panic!("missing edge {from} -> {to} {kind:?}"))
}

#[test]
fn fixture_produces_exact_eventing_nodes() {
    let topology = build_topology(&load_store());
    let expected = vec![
        (NodeCategory::KService, "order-processor".to_owned()),
        (NodeCategory::KService, "order-processor-dls".to_owned()),
        (NodeCategory::Broker, "kafka-broker".to_owned()),
        (NodeCategory::Broker, "mt-broker".to_owned()),
        (NodeCategory::Broker, "rabbitmq-broker".to_owned()),
        (NodeCategory::Trigger, "kafka-orders".to_owned()),
        (NodeCategory::Trigger, "order-created".to_owned()),
        (NodeCategory::Trigger, "rabbitmq-orders".to_owned()),
        (NodeCategory::Source, "ping-orders".to_owned()),
        (NodeCategory::SinkBinding, "deployment-binding".to_owned()),
        (NodeCategory::Subscription, "orders-subscription".to_owned()),
        (NodeCategory::Channel, "mt-broker-kne-trigger".to_owned()),
        (NodeCategory::Channel, "orders-channel".to_owned()),
        (NodeCategory::Channel, "orders-inmemory".to_owned()),
        (NodeCategory::Deployment, "order-consumer".to_owned()),
    ];
    assert_eq!(node_keys(&topology), expected);
}

#[test]
fn fixture_produces_exact_eventing_edges() {
    let topology = build_topology(&load_store());
    let mut expected = vec![
        triple(
            eventing_id("Broker", "mt-broker"),
            messaging_id("InMemoryChannel", "mt-broker-kne-trigger"),
            EdgeKind::BackedBy,
        ),
        triple(
            eventing_id("Broker", "mt-broker"),
            eventing_id("Trigger", "order-created"),
            EdgeKind::DeliversTo,
        ),
        triple(
            eventing_id("Broker", "kafka-broker"),
            eventing_id("Trigger", "kafka-orders"),
            EdgeKind::DeliversTo,
        ),
        triple(
            eventing_id("Broker", "rabbitmq-broker"),
            eventing_id("Trigger", "rabbitmq-orders"),
            EdgeKind::DeliversTo,
        ),
        triple(
            eventing_id("Trigger", "order-created"),
            serving_id("Service", "order-processor"),
            EdgeKind::DeliversTo,
        ),
        triple(
            eventing_id("Trigger", "order-created"),
            serving_id("Service", "order-processor-dls"),
            EdgeKind::DeadLetterSink,
        ),
        triple(
            eventing_id("Trigger", "kafka-orders"),
            serving_id("Service", "order-processor"),
            EdgeKind::DeliversTo,
        ),
        triple(
            eventing_id("Trigger", "rabbitmq-orders"),
            serving_id("Service", "order-processor"),
            EdgeKind::DeliversTo,
        ),
        triple(
            source_id("PingSource", "ping-orders"),
            eventing_id("Broker", "mt-broker"),
            EdgeKind::DeliversTo,
        ),
        triple(
            source_id("SinkBinding", "deployment-binding"),
            eventing_id("Broker", "mt-broker"),
            EdgeKind::DeliversTo,
        ),
        triple(
            apps_id("Deployment", "order-consumer"),
            source_id("SinkBinding", "deployment-binding"),
            EdgeKind::References,
        ),
        triple(
            messaging_id("Channel", "orders-channel"),
            messaging_id("InMemoryChannel", "orders-inmemory"),
            EdgeKind::BackedBy,
        ),
        triple(
            messaging_id("Channel", "orders-channel"),
            messaging_id("Subscription", "orders-subscription"),
            EdgeKind::Subscribes,
        ),
        triple(
            messaging_id("Subscription", "orders-subscription"),
            serving_id("Service", "order-processor"),
            EdgeKind::DeliversTo,
        ),
        triple(
            messaging_id("Subscription", "orders-subscription"),
            serving_id("Service", "order-processor-dls"),
            EdgeKind::DeadLetterSink,
        ),
    ];
    expected.sort();
    assert_eq!(edge_triples(&topology), expected);
}

#[test]
fn trigger_filter_and_dead_letter_metadata_are_recorded() {
    let topology = build_topology(&load_store());
    let filter_edge = edge(
        &topology,
        "mt-broker",
        "order-created",
        EdgeKind::DeliversTo,
    );
    let filter = filter_edge
        .metadata
        .get("filter")
        .expect("filter metadata present");
    assert_eq!(
        filter.get("type").and_then(Value::as_str),
        Some("com.knect.order.created")
    );
    assert_eq!(
        filter.get("source").and_then(Value::as_str),
        Some("/knect/orders")
    );
    assert_eq!(
        filter_edge.metadata.get("filter_count"),
        Some(&serde_json::json!(3))
    );
    assert!(
        badge_labels(node(&topology, NodeCategory::Trigger, "order-created")).contains(&"filtered")
    );

    let dls = edge(
        &topology,
        "order-created",
        "order-processor-dls",
        EdgeKind::DeadLetterSink,
    );
    assert_eq!(
        dls.metadata.get("uri").and_then(Value::as_str),
        Some("http://order-processor-dls.knect-e2e.svc.cluster.local")
    );

    let subscription = edge(
        &topology,
        "orders-subscription",
        "order-processor",
        EdgeKind::DeliversTo,
    );
    assert_eq!(
        subscription
            .metadata
            .get("reply")
            .and_then(|reply| reply.get("ref"))
            .and_then(|reference| reference.get("name"))
            .and_then(Value::as_str),
        Some("mt-broker")
    );
}

#[test]
fn kafka_and_rabbitmq_brokers_do_not_get_channels() {
    let topology = build_topology(&load_store());
    for broker in ["kafka-broker", "rabbitmq-broker"] {
        assert!(
            !topology
                .edges
                .iter()
                .any(|edge| { edge.kind == EdgeKind::BackedBy && edge.from.name == broker }),
            "{broker} must not gain a synthetic channel edge"
        );
    }
    assert!(
        !topology
            .nodes
            .iter()
            .any(|node| node.category == NodeCategory::BackingService)
    );
    let channel = edge(
        &topology,
        "mt-broker",
        "mt-broker-kne-trigger",
        EdgeKind::BackedBy,
    );
    assert_eq!(
        channel.metadata.get("class").and_then(Value::as_str),
        Some("MTChannelBasedBroker")
    );
}

#[test]
fn repeated_runs_are_deterministic() {
    let store = load_store();
    assert_eq!(build_topology(&store), build_topology(&store));
}
