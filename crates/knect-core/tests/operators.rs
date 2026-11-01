#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Fixture-driven backing operator CRD correlation tests.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use knect_core::correlate::eventing::EventingCorrelator;
use knect_core::correlate::operators::OperatorsCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::store::ResourceStore;
use knect_proto::graph::{Edge, EdgeKind, Health, ResourceNode, Topology};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Value, json};

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

fn load_store() -> ResourceStore {
    let contents = std::fs::read_to_string(fixture_path("operators/operators.json"))
        .expect("operators fixture is readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("operators fixture is valid JSON");
    let mut store = ResourceStore::new();
    for object in objects {
        store.upsert(object);
    }
    store
}

fn correlation_context(store: &ResourceStore) -> CorrelationContext {
    CorrelationContext::new("fixture", 1, Arc::new(store.clone()))
}

fn build_topology(store: &ResourceStore) -> Topology {
    let ctx = correlation_context(store);
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    EventingCorrelator
        .correlate(&ctx, &mut graph)
        .expect("eventing correlation succeeds");
    OperatorsCorrelator
        .correlate(&ctx, &mut graph)
        .expect("operators correlation succeeds");
    graph.build()
}

fn backing_nodes(topology: &Topology) -> Vec<(NodeCategory, String)> {
    let mut nodes: Vec<(NodeCategory, String)> = topology
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.category,
                NodeCategory::BackingService | NodeCategory::Topic
            )
        })
        .map(|node| (node.category, node.name.clone()))
        .collect();
    nodes.sort();
    nodes
}

fn node<'a>(topology: &'a Topology, name: &str) -> &'a ResourceNode {
    topology
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.category,
                NodeCategory::BackingService | NodeCategory::Topic
            ) && node.name == name
        })
        .unwrap_or_else(|| panic!("missing operator node {name}"))
}

fn badge_labels(node: &ResourceNode) -> Vec<&str> {
    node.badges
        .iter()
        .map(|badge| badge.label.as_str())
        .collect()
}

fn edge<'a>(topology: &'a Topology, from_name: &str, to_name: &str, kind: EdgeKind) -> &'a Edge {
    topology
        .edges
        .iter()
        .find(|edge| edge.kind == kind && edge.from.name == from_name && edge.to.name == to_name)
        .unwrap_or_else(|| panic!("missing edge {from_name} -> {to_name} {kind:?}"))
}

fn edges_from<'a>(topology: &'a Topology, from_name: &str, kind: EdgeKind) -> Vec<&'a Edge> {
    topology
        .edges
        .iter()
        .filter(|edge| edge.kind == kind && edge.from.name == from_name)
        .collect()
}

fn operator_id(api_version: &str, kind: &str, name: &str) -> ResourceId {
    ResourceId::namespaced(api_version, kind, "knect-e2e", name)
}

#[test]
fn fixture_produces_one_node_per_operator_family() {
    let topology = build_topology(&load_store());
    let expected = vec![
        (NodeCategory::BackingService, "dragonfly".to_owned()),
        (NodeCategory::BackingService, "orders-kafka".to_owned()),
        (NodeCategory::BackingService, "postgres".to_owned()),
        (NodeCategory::BackingService, "rabbitmq-broker".to_owned()),
        (NodeCategory::BackingService, "redis".to_owned()),
        (NodeCategory::BackingService, "redpanda".to_owned()),
        (
            NodeCategory::Topic,
            "knative-broker-knect-e2e-redpanda".to_owned(),
        ),
        (NodeCategory::Topic, "orders".to_owned()),
    ];
    assert_eq!(backing_nodes(&topology), expected);
}

#[test]
fn health_and_capacity_metadata_are_extracted() {
    let topology = build_topology(&load_store());

    let kafka = node(&topology, "orders-kafka");
    assert_eq!(kafka.health, Health::Healthy);
    assert!(kafka.summary.contains("3/3 replicas"));
    assert!(badge_labels(kafka).contains(&"ready"));
    assert!(badge_labels(kafka).contains(&"3/3 replicas"));

    let topic = node(&topology, "orders");
    assert_eq!(topic.health, Health::Healthy);
    assert!(topic.summary.contains("12 partitions"));
    assert!(topic.summary.contains("RF 3"));
    assert!(badge_labels(topic).contains(&"12 partitions"));
    assert_eq!(
        topic
            .badges
            .iter()
            .find(|badge| badge.label == "12 partitions")
            .and_then(|badge| badge.tooltip.as_deref()),
        Some("replication factor 3")
    );

    let redpanda_topic = node(&topology, "knative-broker-knect-e2e-redpanda");
    assert_eq!(redpanda_topic.health, Health::Degraded);
    assert!(redpanda_topic.summary.contains("6 partitions"));
    assert!(badge_labels(redpanda_topic).contains(&"degraded"));

    let rabbitmq = node(&topology, "rabbitmq-broker");
    assert_eq!(rabbitmq.health, Health::Healthy);
    assert!(rabbitmq.summary.contains("3/3 replicas"));
    assert!(badge_labels(rabbitmq).contains(&"queue orders"));
    assert!(badge_labels(rabbitmq).contains(&"exchange knect"));

    let dragonfly = node(&topology, "dragonfly");
    assert_eq!(dragonfly.health, Health::Healthy);
    assert!(dragonfly.summary.contains("2/2 replicas"));

    let redis = node(&topology, "redis");
    assert_eq!(redis.health, Health::Degraded);
    assert!(redis.summary.contains("2/3 replicas"));
    assert!(badge_labels(redis).contains(&"degraded"));

    let postgres = node(&topology, "postgres");
    assert_eq!(postgres.health, Health::Healthy);
    assert!(postgres.summary.contains("3/3 instances"));
    assert!(postgres.summary.contains("Cluster in healthy state"));
    assert!(badge_labels(postgres).contains(&"ready"));
}

#[test]
fn kafka_broker_links_to_strimzi_cluster_and_topic() {
    let topology = build_topology(&load_store());

    let backed = edge(
        &topology,
        "orders-kafka",
        "orders-kafka",
        EdgeKind::BackedBy,
    );
    assert_eq!(backed.health, Health::Healthy);
    assert_eq!(
        backed.to,
        operator_id("kafka.strimzi.io/v1beta2", "Kafka", "orders-kafka")
    );
    assert_eq!(
        backed.metadata.get("class").and_then(Value::as_str),
        Some("Kafka")
    );
    assert_eq!(
        backed.metadata.get("family").and_then(Value::as_str),
        Some("Kafka")
    );
    assert_eq!(backed.metadata.get("replicas"), Some(&json!(3)));

    let consumed = edge(&topology, "orders-kafka", "orders", EdgeKind::ConsumesFrom);
    assert_eq!(consumed.health, Health::Healthy);
    assert_eq!(
        consumed.to,
        operator_id("kafka.strimzi.io/v1beta2", "KafkaTopic", "orders")
    );
    assert_eq!(consumed.metadata.get("partitions"), Some(&json!(12)));
    assert_eq!(consumed.metadata.get("replication_factor"), Some(&json!(3)));
}

#[test]
fn kafka_broker_links_to_redpanda_cluster_and_topic() {
    let topology = build_topology(&load_store());

    let backed = edge(&topology, "redpanda-kafka", "redpanda", EdgeKind::BackedBy);
    assert_eq!(
        backed.to,
        operator_id("cluster.redpanda.com/v1alpha2", "Cluster", "redpanda")
    );
    assert_eq!(
        backed.metadata.get("family").and_then(Value::as_str),
        Some("Redpanda Cluster")
    );

    let consumed = edge(
        &topology,
        "redpanda-kafka",
        "knative-broker-knect-e2e-redpanda",
        EdgeKind::ConsumesFrom,
    );
    assert_eq!(consumed.health, Health::Degraded);
    assert_eq!(
        consumed.to,
        operator_id(
            "cluster.redpanda.com/v1alpha1",
            "Topic",
            "knative-broker-knect-e2e-redpanda"
        )
    );
    assert_eq!(consumed.metadata.get("partitions"), Some(&json!(6)));
}

#[test]
fn rabbitmq_broker_links_to_its_cluster_without_a_channel() {
    let topology = build_topology(&load_store());

    let backed = edge(
        &topology,
        "orders-rabbitmq",
        "rabbitmq-broker",
        EdgeKind::BackedBy,
    );
    assert_eq!(
        backed.to,
        operator_id("rabbitmq.com/v1beta1", "RabbitmqCluster", "rabbitmq-broker")
    );
    assert_eq!(
        backed.metadata.get("family").and_then(Value::as_str),
        Some("RabbitmqCluster")
    );
    assert!(
        edges_from(&topology, "orders-rabbitmq", EdgeKind::ConsumesFrom).is_empty(),
        "RabbitMQ brokers do not consume from a topic CRD"
    );
    assert!(
        !topology.nodes.iter().any(|node| {
            node.category == NodeCategory::Channel && node.name.contains("rabbitmq")
        }),
        "RabbitMQ brokers must not gain a synthetic Channel"
    );
}

#[test]
fn topics_are_backed_by_their_cluster() {
    let topology = build_topology(&load_store());

    let strimzi = edge(&topology, "orders", "orders-kafka", EdgeKind::BackedBy);
    assert_eq!(strimzi.health, Health::Healthy);
    assert_eq!(strimzi.metadata.get("partitions"), Some(&json!(12)));

    let redpanda = edge(
        &topology,
        "knative-broker-knect-e2e-redpanda",
        "redpanda",
        EdgeKind::BackedBy,
    );
    assert_eq!(
        redpanda
            .metadata
            .get("cluster_family")
            .and_then(Value::as_str),
        Some("Redpanda Cluster")
    );
}

#[test]
fn service_nodes_link_to_backing_crds() {
    let store = load_store();
    let ctx = correlation_context(&store);
    let service_id =
        ResourceId::namespaced("v1", "Service", "knect-e2e", "orders-kafka-kafka-bootstrap");
    let now = Utc::now();
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    graph.add_node(ResourceNode {
        id: service_id.clone(),
        category: NodeCategory::Service,
        name: service_id.name.clone(),
        namespace: service_id.namespace.clone(),
        health: Health::Unknown,
        summary: "fixture service".to_owned(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
        badges: Vec::new(),
        uid: None,
        first_seen: now,
        last_seen: now,
    });
    OperatorsCorrelator
        .correlate(&ctx, &mut graph)
        .expect("operators correlation succeeds");
    let topology = graph.build();

    let backed = edge(
        &topology,
        "orders-kafka-kafka-bootstrap",
        "orders-kafka",
        EdgeKind::BackedBy,
    );
    assert_eq!(backed.from, service_id);
    assert_eq!(
        backed.to,
        operator_id("kafka.strimzi.io/v1beta2", "Kafka", "orders-kafka")
    );
    assert_eq!(backed.health, Health::Healthy);
}

#[test]
fn missing_crds_create_no_nodes_and_no_edges() {
    let mut store = load_store();
    store.remove(&operator_id(
        "kafka.strimzi.io/v1beta2",
        "Kafka",
        "orders-kafka",
    ));
    store.remove(&operator_id(
        "kafka.strimzi.io/v1beta2",
        "KafkaTopic",
        "orders",
    ));
    store.remove(&operator_id(
        "rabbitmq.com/v1beta1",
        "RabbitmqCluster",
        "rabbitmq-broker",
    ));

    let topology = build_topology(&store);

    assert!(
        !topology.nodes.iter().any(|node| node.name == "orders-kafka"
            && matches!(
                node.category,
                NodeCategory::BackingService | NodeCategory::Topic
            ))
    );
    assert!(
        !topology
            .nodes
            .iter()
            .any(|node| node.name == "orders" && node.category == NodeCategory::Topic)
    );
    assert!(!topology.nodes.iter().any(
        |node| node.name == "rabbitmq-broker" && node.category == NodeCategory::BackingService
    ));

    assert!(
        edges_from(&topology, "orders-kafka", EdgeKind::BackedBy).is_empty(),
        "missing Kafka must not produce a BackedBy edge"
    );
    assert!(
        edges_from(&topology, "orders-kafka", EdgeKind::ConsumesFrom).is_empty(),
        "missing KafkaTopic must not produce a ConsumesFrom edge"
    );
    assert!(
        edges_from(&topology, "orders-rabbitmq", EdgeKind::BackedBy).is_empty(),
        "missing RabbitmqCluster must not produce a BackedBy edge"
    );
}

#[test]
fn repeated_runs_are_deterministic() {
    let store = load_store();
    assert_eq!(build_topology(&store), build_topology(&store));
}

#[test]
fn topology_never_contains_raw_secret_material() {
    let topology = build_topology(&load_store());
    let serialized = serde_json::to_string(&topology).expect("topology serializes");

    assert!(!serialized.contains("hunter2-knect-sentinel"));
    assert!(!serialized.contains("aHVudGVyMi1rbmVjdC1zZW50aW5lbA=="));

    let kafka = node(&topology, "orders-kafka");
    assert_eq!(
        kafka
            .annotations
            .get("strimzi.io/cluster-credentials")
            .map(String::as_str),
        Some("<redacted>")
    );
}
