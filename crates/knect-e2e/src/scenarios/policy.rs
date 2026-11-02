//! `policy` scenario.
//!
//! Verifies Istio, Flagger, and Kyverno correlation and that a validation
//! failure on a Revision/Pod is surfaced on the parent Knative Service.

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, NodeDetails, Severity};
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{
    ScenarioError, badge_labels, fixture, protocol, require_edge, require_node,
};

/// Runs the policy scenario against the isolated policy fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("policy").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::VirtualService, "order-processor-custom"),
        (NodeCategory::DestinationRule, "order-processor"),
        (NodeCategory::PeerAuthentication, "order-processor-strict"),
        (NodeCategory::AuthorizationPolicy, "allow-order-processor"),
        (NodeCategory::Canary, "order-processor-flagger"),
        (NodeCategory::Canary, "checkout-canary"),
        (NodeCategory::Policy, "knect-e2e-require-labels"),
    ] {
        require_node(&topology, category, name)?;
    }

    let progressing = require_node(&topology, NodeCategory::Canary, "order-processor-flagger")?;
    if !progressing.summary.contains("Progressing")
        || !progressing.summary.contains("25/50% weight")
        || !progressing.summary.contains("3 iterations")
    {
        return Err(protocol(format!(
            "Flagger Canary status is incomplete: {}",
            progressing.summary
        )));
    }
    let failed = require_node(&topology, NodeCategory::Canary, "checkout-canary")?;
    if !failed.summary.contains("Failed") || !failed.summary.contains("2 failed checks") {
        return Err(protocol(format!(
            "failed Flagger Canary status is incomplete: {}",
            failed.summary
        )));
    }

    let conflict = topology
        .warnings
        .iter()
        .find(|warning| warning.code == "istio_host_conflict")
        .ok_or_else(|| protocol("no Istio host-conflict warning"))?;
    if conflict.severity != Severity::High {
        return Err(protocol("Istio host conflict is not high severity"));
    }
    let custom = require_node(
        &topology,
        NodeCategory::VirtualService,
        "order-processor-custom",
    )?;
    if !badge_labels(custom).contains(&"host-conflict") {
        return Err(protocol(
            "custom VirtualService is missing the host-conflict badge",
        ));
    }

    require_edge(
        &topology,
        "order-processor-flagger",
        "order-processor",
        EdgeKind::Targets,
    )?;
    require_edge(
        &topology,
        "order-processor-00001",
        "allow-order-processor",
        EdgeKind::ProtectedBy,
    )?;

    let service = require_node(&topology, NodeCategory::KService, "order-processor")?;
    let violation = topology
        .warnings
        .iter()
        .find(|warning| {
            warning.code == "policy_violation" && warning.resource.as_ref() == Some(&service.id)
        })
        .ok_or_else(|| protocol("Kyverno/Pod violation is not attached to the KService"))?;
    if violation.severity != Severity::High {
        return Err(protocol("KService policy violation is not high severity"));
    }
    if !violation.message.contains("require-part-of-label") {
        return Err(protocol(format!(
            "KService policy violation names the wrong rule: {}",
            violation.message
        )));
    }
    for sentinel in ["hunter2-sentinel", "event-sentinel", "secretdata-sentinel"] {
        if violation.message.contains(sentinel) {
            return Err(protocol("policy violation leaked a Secret value"));
        }
    }

    let details: NodeDetails = serde_json::from_value(
        harness
            .call(
                "graph.get_node_details",
                Some(json!({"id": service.id.clone(), "include_yaml": false})),
            )
            .await?,
    )
    .map_err(|error| protocol(format!("invalid node details: {error}")))?;
    if details.policies.is_empty() {
        return Err(protocol("KService node details have no policy records"));
    }
    let has_kyverno = details
        .policies
        .iter()
        .any(|policy| policy.kind == knect_proto::policy::PolicyKind::Kyverno);
    if !has_kyverno {
        return Err(protocol(
            "KService node details have no Kyverno policy record",
        ));
    }

    daemon.shutdown().await?;
    Ok(())
}
