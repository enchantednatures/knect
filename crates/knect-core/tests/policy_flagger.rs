#![allow(clippy::unwrap_used, clippy::expect_used)]

mod policy_support;

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, Health};
use serde_json::json;

use policy_support::{badge_labels, build_istio_flagger, edge, load_store, node, policy_node_keys};

#[test]
fn flagger_family_projects_canary_nodes() {
    let topology = build_istio_flagger(&load_store());
    let policy_nodes = policy_node_keys(&topology);
    let canaries: Vec<(NodeCategory, String)> = policy_nodes
        .into_iter()
        .filter(|(category, _)| *category == NodeCategory::Canary)
        .collect();
    assert_eq!(
        canaries,
        vec![
            (NodeCategory::Canary, "checkout-canary".to_owned()),
            (NodeCategory::Canary, "order-processor-flagger".to_owned()),
        ]
    );
}

#[test]
fn flagger_phase_weight_iterations_and_health_are_projected() {
    let topology = build_istio_flagger(&load_store());
    let progressing = node(&topology, NodeCategory::Canary, "order-processor-flagger");
    assert_eq!(progressing.health, Health::Progressing);
    assert!(progressing.summary.contains("Progressing"));
    assert!(progressing.summary.contains("25/50% weight"));
    assert!(progressing.summary.contains("3 iterations"));
    assert!(progressing.summary.contains("0 failed checks"));
    let labels = badge_labels(progressing);
    assert!(labels.contains(&"progressing"));
    assert!(labels.contains(&"canary-weight"));
    assert!(labels.contains(&"metrics"));
    assert_eq!(
        progressing
            .badges
            .iter()
            .find(|badge| badge.label == "canary-weight")
            .and_then(|badge| badge.tooltip.as_deref()),
        Some("25/50%")
    );
    assert_eq!(
        progressing
            .annotations
            .get("knect.dev/metric-templates")
            .map(String::as_str),
        Some("knect-request-success-rate")
    );

    let failed = node(&topology, NodeCategory::Canary, "checkout-canary");
    assert_eq!(failed.health, Health::Degraded);
    assert!(failed.summary.contains("Failed"));
    assert!(failed.summary.contains("50/50% weight"));
    assert!(failed.summary.contains("7 iterations"));
    assert!(failed.summary.contains("2 failed checks"));
    assert!(badge_labels(failed).contains(&"failed"));
    assert!(badge_labels(failed).contains(&"failed-checks"));
}

#[test]
fn flagger_targets_knowledge_service_and_deployment() {
    let topology = build_istio_flagger(&load_store());
    let service_target = edge(
        &topology,
        "order-processor-flagger",
        "order-processor",
        EdgeKind::Targets,
    );
    assert_eq!(
        service_target
            .metadata
            .get("target_kind")
            .and_then(|value| value.as_str()),
        Some("Service")
    );
    assert_eq!(
        service_target
            .metadata
            .get("phase")
            .and_then(|value| value.as_str()),
        Some("Progressing")
    );
    assert_eq!(
        service_target.metadata.get("current_weight"),
        Some(&json!(25))
    );
    assert_eq!(service_target.metadata.get("max_weight"), Some(&json!(50)));
    assert_eq!(service_target.metadata.get("iterations"), Some(&json!(3)));
    assert_eq!(
        service_target.metadata.get("failed_checks"),
        Some(&json!(0))
    );
    assert_eq!(
        service_target.metadata.get("metric_templates"),
        Some(&json!(["knect-request-success-rate"]))
    );
    assert!(
        service_target
            .metadata
            .get("metric_checks")
            .is_some_and(|value| value.is_array())
    );
    assert!(
        service_target
            .metadata
            .get("conditions")
            .is_some_and(|value| value.is_array())
    );

    let deployment_target = edge(
        &topology,
        "checkout-canary",
        "checkout-canary",
        EdgeKind::Targets,
    );
    assert_eq!(
        deployment_target
            .metadata
            .get("target_kind")
            .and_then(|value| value.as_str()),
        Some("Deployment")
    );
    assert_eq!(
        deployment_target.metadata.get("failed_checks"),
        Some(&json!(2))
    );
    assert_eq!(
        deployment_target.metadata.get("current_weight"),
        Some(&json!(50))
    );
}

#[test]
fn flagger_manages_generated_istio_resources() {
    let topology = build_istio_flagger(&load_store());
    let virtual_service = edge(
        &topology,
        "order-processor-flagger",
        "order-processor-flagger",
        EdgeKind::Manages,
    );
    assert_eq!(
        virtual_service
            .metadata
            .get("managed_by")
            .and_then(|value| value.as_str()),
        Some("flagger")
    );
    assert_eq!(
        virtual_service
            .metadata
            .get("kind")
            .and_then(|value| value.as_str()),
        Some("VirtualService")
    );
    for destination_rule in [
        "order-processor-flagger-primary",
        "order-processor-flagger-canary",
    ] {
        let manages = edge(
            &topology,
            "order-processor-flagger",
            destination_rule,
            EdgeKind::Manages,
        );
        assert_eq!(
            manages
                .metadata
                .get("kind")
                .and_then(|value| value.as_str()),
            Some("DestinationRule")
        );
    }
}
