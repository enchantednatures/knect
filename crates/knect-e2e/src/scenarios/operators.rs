//! `operators` scenario.
//!
//! Verifies that backing operator CRDs are projected and that Kafka and
//! RabbitMQ Brokers resolve to the clusters that own them.

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, Health};
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{
    ScenarioError, badge_labels, fixture, protocol, require_edge, require_node,
};

/// Runs the operators scenario against the isolated operator fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("operators").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::BackingService, "orders-kafka"),
        (NodeCategory::BackingService, "redpanda"),
        (NodeCategory::BackingService, "rabbitmq-broker"),
        (NodeCategory::BackingService, "dragonfly"),
        (NodeCategory::BackingService, "redis"),
        (NodeCategory::BackingService, "postgres"),
        (NodeCategory::Topic, "orders"),
        (NodeCategory::Topic, "knative-broker-knect-e2e-redpanda"),
    ] {
        require_node(&topology, category, name)?;
    }

    let kafka = require_node(&topology, NodeCategory::BackingService, "orders-kafka")?;
    if kafka.health != Health::Healthy {
        return Err(protocol(format!(
            "Kafka cluster health is {:?}, expected healthy",
            kafka.health
        )));
    }
    if !kafka.summary.contains("3/3 replicas") {
        return Err(protocol(format!(
            "Kafka summary does not expose replicas: {}",
            kafka.summary
        )));
    }
    let labels = badge_labels(kafka);
    for label in ["ready", "3/3 replicas"] {
        if !labels.contains(&label) {
            return Err(protocol(format!("Kafka is missing badge {label}")));
        }
    }

    let topic = require_node(&topology, NodeCategory::Topic, "orders")?;
    if !topic.summary.contains("12 partitions") || !topic.summary.contains("RF 3") {
        return Err(protocol(format!(
            "KafkaTopic summary lacks capacity metadata: {}",
            topic.summary
        )));
    }

    let rabbitmq = require_node(&topology, NodeCategory::BackingService, "rabbitmq-broker")?;
    if rabbitmq.health != Health::Healthy || !rabbitmq.summary.contains("3/3 replicas") {
        return Err(protocol(format!(
            "RabbitmqCluster did not report healthy capacity: {} / {:?}",
            rabbitmq.summary, rabbitmq.health
        )));
    }

    let kafka_backed = require_edge(
        &topology,
        "orders-kafka",
        "orders-kafka",
        EdgeKind::BackedBy,
    )?;
    if kafka_backed.health != Health::Healthy {
        return Err(protocol("Kafka Broker backed_by edge is not healthy"));
    }
    if kafka_backed.metadata.get("family") != Some(&json!("Kafka")) {
        return Err(protocol("Kafka Broker backed_by edge lacks the family"));
    }
    if kafka_backed.metadata.get("replicas") != Some(&json!(3)) {
        return Err(protocol("Kafka Broker backed_by edge lacks replicas"));
    }

    let consumed = require_edge(&topology, "orders-kafka", "orders", EdgeKind::ConsumesFrom)?;
    if consumed.metadata.get("partitions") != Some(&json!(12))
        || consumed.metadata.get("replication_factor") != Some(&json!(3))
    {
        return Err(protocol(
            "Kafka Broker consumes_from edge lacks partition metadata",
        ));
    }

    require_edge(
        &topology,
        "orders-rabbitmq",
        "rabbitmq-broker",
        EdgeKind::BackedBy,
    )?;
    require_edge(&topology, "redpanda-kafka", "redpanda", EdgeKind::BackedBy)?;

    daemon.shutdown().await?;
    Ok(())
}
