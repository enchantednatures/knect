//! `links` scenario.
//!
//! Verifies resolved service/secret/config-map links, unresolved-reference
//! warnings, and that Secret material never leaves the daemon.

use knect_proto::NodeCategory;
use knect_proto::graph::EdgeKind;
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, fixture, protocol, require_edge, require_node};

/// Runs the links scenario against the isolated links fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("links").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::KService, "order-processor"),
        (NodeCategory::Revision, "order-processor-00001"),
        (NodeCategory::Service, "kafka-bootstrap"),
        (NodeCategory::Service, "postgres-e2e-rw"),
        (NodeCategory::Service, "redis-master"),
        (NodeCategory::BackingService, "redis-cache"),
        (NodeCategory::Secret, "redis-credentials"),
        (NodeCategory::ConfigMap, "order-config"),
        (NodeCategory::SinkBinding, "order-sink-binding"),
    ] {
        require_node(&topology, category, name)?;
    }

    require_edge(
        &topology,
        "order-processor-00001",
        "redis-cache",
        EdgeKind::ConnectsTo,
    )?;
    require_edge(
        &topology,
        "order-processor-00001",
        "redis-credentials",
        EdgeKind::UsesSecret,
    )?;
    require_edge(
        &topology,
        "order-processor-00001",
        "order-config",
        EdgeKind::UsesConfigMap,
    )?;
    require_edge(
        &topology,
        "order-processor-00001",
        "order-sink-binding",
        EdgeKind::References,
    )?;

    let unresolved = topology
        .warnings
        .iter()
        .find(|warning| warning.code == "link_target_unresolved")
        .ok_or_else(|| protocol("no unresolved link warning"))?;
    if !unresolved.message.contains("ghost-db") {
        return Err(protocol(format!(
            "unresolved warning does not name ghost-db: {}",
            unresolved.message
        )));
    }

    let serialized = serde_json::to_string(&topology)
        .map_err(|error| protocol(format!("cannot serialize topology: {error}")))?;
    if serialized.contains("hunter2-knect-sentinel") {
        return Err(protocol("topology serialization leaked Secret material"));
    }

    let revision = require_node(&topology, NodeCategory::Revision, "order-processor-00001")?;
    let details = harness
        .call(
            "graph.get_node_details",
            Some(json!({"id": revision.id.clone(), "include_yaml": true})),
        )
        .await?;
    let rendered = serde_json::to_string(&details)
        .map_err(|error| protocol(format!("cannot serialize details: {error}")))?;
    if rendered.contains("hunter2-knect-sentinel") {
        return Err(protocol("node details leaked Secret material"));
    }
    if details["links"].as_array().is_none_or(Vec::is_empty) {
        return Err(protocol("node details have no links"));
    }

    daemon.shutdown().await?;
    Ok(())
}
