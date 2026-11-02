//! `eventing` scenario.
//!
//! Verifies Broker/Trigger/source/channel correlation, filter metadata, and
//! dead-letter delivery through the daemon topology API.

use knect_proto::NodeCategory;
use knect_proto::graph::EdgeKind;
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, fixture, protocol, require_edge, require_node};

/// Runs the eventing scenario against the isolated eventing fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("eventing").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::Broker, "kafka-broker"),
        (NodeCategory::Broker, "rabbitmq-broker"),
        (NodeCategory::Broker, "mt-broker"),
        (NodeCategory::Trigger, "kafka-orders"),
        (NodeCategory::Trigger, "rabbitmq-orders"),
        (NodeCategory::Trigger, "order-created"),
        (NodeCategory::Source, "ping-orders"),
        (NodeCategory::SinkBinding, "deployment-binding"),
        (NodeCategory::Channel, "orders-channel"),
        (NodeCategory::Subscription, "orders-subscription"),
    ] {
        require_node(&topology, category, name)?;
    }

    let broker = require_node(&topology, NodeCategory::Broker, "mt-broker")?;
    if !broker.summary.contains("MTChannelBasedBroker") {
        return Err(protocol(format!(
            "Broker summary lost its class: {}",
            broker.summary
        )));
    }

    let edge = require_edge(
        &topology,
        "mt-broker",
        "mt-broker-kne-trigger",
        EdgeKind::BackedBy,
    )?;
    if edge.health != knect_proto::graph::Health::Healthy {
        return Err(protocol("MTChannelBasedBroker backing edge is not healthy"));
    }

    let trigger = require_edge(
        &topology,
        "kafka-broker",
        "kafka-orders",
        EdgeKind::DeliversTo,
    )?;
    if trigger.metadata.get("filter_count") != Some(&json!(1)) {
        return Err(protocol("Kafka trigger filter metadata is missing"));
    }

    let filtered = require_edge(
        &topology,
        "mt-broker",
        "order-created",
        EdgeKind::DeliversTo,
    )?;
    if filtered.metadata.get("filter_count") != Some(&json!(3)) {
        return Err(protocol("Trigger filter_count is not 3"));
    }
    let filters = filtered
        .metadata
        .get("filter")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| protocol("Trigger filter metadata is not an object"))?;
    for (key, value) in [
        ("type", "com.knect.order.created"),
        ("source", "/knect/orders"),
        ("knecttenant", "acme"),
    ] {
        if filters.get(key).and_then(serde_json::Value::as_str) != Some(value) {
            return Err(protocol(format!("Trigger filter {key} is not {value}")));
        }
    }

    require_edge(
        &topology,
        "order-created",
        "order-processor-dls",
        EdgeKind::DeadLetterSink,
    )?;
    require_edge(&topology, "ping-orders", "mt-broker", EdgeKind::DeliversTo)?;
    require_edge(
        &topology,
        "deployment-binding",
        "mt-broker",
        EdgeKind::DeliversTo,
    )?;
    require_edge(
        &topology,
        "orders-channel",
        "orders-subscription",
        EdgeKind::Subscribes,
    )?;

    daemon.shutdown().await?;
    Ok(())
}
