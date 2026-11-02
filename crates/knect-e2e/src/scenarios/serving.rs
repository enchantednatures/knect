//! `serving` scenario.
//!
//! Verifies the complete Knative Serving owner chain and revision scale states
//! through the daemon topology API.

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, Health};
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{
    ScenarioError, badge_labels, fixture, protocol, require_edge, require_node,
};

/// Runs the serving scenario against the isolated serving fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("serving").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::KService, "order-processor"),
        (NodeCategory::Route, "order-processor"),
        (NodeCategory::Configuration, "order-processor"),
        (NodeCategory::Revision, "order-processor-00001"),
        (NodeCategory::Revision, "order-processor-00002"),
        (NodeCategory::Revision, "order-processor-00003"),
        (NodeCategory::PodAutoscaler, "order-processor-00001"),
        (NodeCategory::Deployment, "order-processor-00001-deployment"),
        (NodeCategory::Pod, "order-processor-00001-abc"),
        (NodeCategory::KService, "inventory-service"),
    ] {
        require_node(&topology, category, name)?;
    }

    require_edge(
        &topology,
        "order-processor",
        "order-processor",
        EdgeKind::Owns,
    )?;
    require_edge(
        &topology,
        "order-processor",
        "order-processor-00001",
        EdgeKind::RoutesTo,
    )?;
    require_edge(
        &topology,
        "order-processor-00001",
        "order-processor-00001",
        EdgeKind::Scales,
    )?;
    require_edge(
        &topology,
        "order-processor-00001-deployment",
        "order-processor-00001-abc",
        EdgeKind::Runs,
    )?;

    let active = require_node(&topology, NodeCategory::Revision, "order-processor-00001")?;
    if active.health != Health::Healthy || !badge_labels(active).contains(&"active") {
        return Err(protocol("revision 00001 is not active/healthy"));
    }
    let scaled = require_node(&topology, NodeCategory::Revision, "order-processor-00002")?;
    if !badge_labels(scaled).contains(&"scaled-to-zero") {
        return Err(protocol("revision 00002 is not scaled to zero"));
    }
    let broken = require_node(&topology, NodeCategory::Revision, "order-processor-00003")?;
    if broken.health != Health::Degraded
        || !badge_labels(broken).contains(&"crashloop")
        || !broken.summary.contains("ProgressDeadlineExceeded")
    {
        return Err(protocol(format!(
            "revision 00003 does not report its crash-loop state: {}",
            broken.summary
        )));
    }

    let route = require_node(&topology, NodeCategory::Route, "order-processor")?;
    if !route.summary.contains("90%") || !route.summary.contains("10%") {
        return Err(protocol(format!(
            "route traffic split is incomplete: {}",
            route.summary
        )));
    }

    let details = harness
        .call(
            "graph.get_node_details",
            Some(json!({"id": active.id, "include_yaml": true})),
        )
        .await?;
    if details["yaml_redacted"] != true {
        return Err(protocol("revision details did not mark YAML redacted"));
    }
    if !details["yaml"]
        .as_str()
        .is_some_and(|yaml| yaml.contains("kind: Revision"))
    {
        return Err(protocol("revision details did not include YAML"));
    }

    daemon.shutdown().await?;
    Ok(())
}
