#![allow(clippy::unwrap_used, clippy::expect_used)]

mod policy_support;

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, Health, Severity};
use serde_json::json;

use policy_support::{
    badge_labels, build_istio, edge, edge_triples, istio_id, load_store, node, policy_node_keys,
    security_id, serving_id,
};

#[test]
fn istio_family_projects_virtual_service_destination_rule_and_security_nodes() {
    let topology = build_istio(&load_store());
    let mut expected = vec![
        (
            NodeCategory::VirtualService,
            "order-processor-custom".to_owned(),
        ),
        (
            NodeCategory::VirtualService,
            "order-processor-external".to_owned(),
        ),
        (
            NodeCategory::VirtualService,
            "order-processor-flagger".to_owned(),
        ),
        (
            NodeCategory::VirtualService,
            "order-processor-ingress".to_owned(),
        ),
        (NodeCategory::DestinationRule, "order-processor".to_owned()),
        (
            NodeCategory::DestinationRule,
            "order-processor-flagger-canary".to_owned(),
        ),
        (
            NodeCategory::DestinationRule,
            "order-processor-flagger-primary".to_owned(),
        ),
        (
            NodeCategory::PeerAuthentication,
            "knect-e2e-permissive".to_owned(),
        ),
        (
            NodeCategory::PeerAuthentication,
            "order-processor-strict".to_owned(),
        ),
        (
            NodeCategory::AuthorizationPolicy,
            "allow-order-processor".to_owned(),
        ),
    ];
    expected.sort();
    assert_eq!(policy_node_keys(&topology), expected);
}

#[test]
fn istio_family_links_traffic_security_and_destination_rules() {
    let topology = build_istio(&load_store());
    let triples = edge_triples(&topology);
    for expected in [
        (
            istio_id("VirtualService", "order-processor-ingress"),
            serving_id("Route", "order-processor"),
            EdgeKind::Manages,
        ),
        (
            istio_id("VirtualService", "order-processor-ingress"),
            serving_id("Revision", "order-processor-00001"),
            EdgeKind::Targets,
        ),
        (
            istio_id("VirtualService", "order-processor-ingress"),
            serving_id("Revision", "order-processor-00002"),
            EdgeKind::Targets,
        ),
        (
            istio_id("DestinationRule", "order-processor"),
            serving_id("Service", "order-processor"),
            EdgeKind::Targets,
        ),
        (
            serving_id("Service", "order-processor"),
            security_id("PeerAuthentication", "order-processor-strict"),
            EdgeKind::ProtectedBy,
        ),
        (
            serving_id("Service", "order-processor"),
            security_id("AuthorizationPolicy", "allow-order-processor"),
            EdgeKind::ProtectedBy,
        ),
    ] {
        assert!(
            triples.contains(&expected),
            "missing expected edge {expected:?}"
        );
    }
    assert!(
        topology
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Manages
                && edge.from.name == "order-processor-ingress"
                && edge.to.kind == "Service")
    );
    assert!(
        topology
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Targets
                && edge.from.kind == "DestinationRule"
                && edge.to.kind == "Service")
    );
}

#[test]
fn custom_virtual_service_host_conflict_is_high_severity_with_metadata() {
    let topology = build_istio(&load_store());
    let conflicts: Vec<_> = topology
        .warnings
        .iter()
        .filter(|warning| warning.code == "istio_host_conflict")
        .collect();
    assert_eq!(conflicts.len(), 1);
    let conflict = conflicts[0];
    assert_eq!(conflict.severity, Severity::High);
    assert_eq!(
        conflict
            .resource
            .as_ref()
            .map(|resource| resource.kind.as_str()),
        Some("Route")
    );
    assert!(conflict.message.contains("order-processor-custom"));
    assert!(
        conflict
            .message
            .contains("order-processor.knect-e2e.sslip.io")
    );

    let custom = node(
        &topology,
        NodeCategory::VirtualService,
        "order-processor-custom",
    );
    assert!(badge_labels(custom).contains(&"host-conflict"));
    let conflict_badge = custom
        .badges
        .iter()
        .find(|badge| badge.label == "host-conflict")
        .expect("conflict badge");
    assert_eq!(conflict_badge.severity, Severity::High);
    assert!(custom.summary.contains("host conflict"));

    let manages = edge(
        &topology,
        "order-processor-custom",
        "order-processor",
        EdgeKind::Manages,
    );
    assert_eq!(manages.metadata.get("host_conflict"), Some(&json!(true)));
    assert_eq!(
        manages
            .metadata
            .get("conflict_host")
            .and_then(|value| value.as_str()),
        Some("order-processor.knect-e2e.sslip.io")
    );
    assert!(manages.metadata.contains_key("conflict_with"));
}

#[test]
fn managed_virtual_services_do_not_conflict() {
    let topology = build_istio(&load_store());
    let managed = node(
        &topology,
        NodeCategory::VirtualService,
        "order-processor-ingress",
    );
    assert!(badge_labels(managed).contains(&"knative"));
    assert!(!badge_labels(managed).contains(&"host-conflict"));
    let external = node(
        &topology,
        NodeCategory::VirtualService,
        "order-processor-external",
    );
    assert!(badge_labels(external).contains(&"custom"));
    assert!(!badge_labels(external).contains(&"host-conflict"));
}

#[test]
fn mtls_mode_and_authorization_action_are_recorded() {
    let topology = build_istio(&load_store());
    let strict = node(
        &topology,
        NodeCategory::PeerAuthentication,
        "order-processor-strict",
    );
    assert_eq!(strict.health, Health::Healthy);
    assert!(strict.summary.contains("STRICT"));
    assert!(badge_labels(strict).contains(&"mtls-strict"));
    let strict_edge = edge(
        &topology,
        "order-processor",
        "order-processor-strict",
        EdgeKind::ProtectedBy,
    );
    assert_eq!(
        strict_edge
            .metadata
            .get("mtls_mode")
            .and_then(|value| value.as_str()),
        Some("STRICT")
    );
    assert_eq!(
        strict_edge
            .metadata
            .get("scope")
            .and_then(|value| value.as_str()),
        Some("selector")
    );

    let allow = node(
        &topology,
        NodeCategory::AuthorizationPolicy,
        "allow-order-processor",
    );
    assert!(allow.summary.contains("ALLOW"));
    assert!(badge_labels(allow).contains(&"action-allow"));
    let allow_edge = edge(
        &topology,
        "order-processor",
        "allow-order-processor",
        EdgeKind::ProtectedBy,
    );
    assert_eq!(
        allow_edge
            .metadata
            .get("action")
            .and_then(|value| value.as_str()),
        Some("ALLOW")
    );
    assert_eq!(allow_edge.metadata.get("rules_count"), Some(&json!(1)));
}

#[test]
fn destination_rules_record_subsets_and_tls_mode() {
    let topology = build_istio(&load_store());
    let rule = node(&topology, NodeCategory::DestinationRule, "order-processor");
    assert!(rule.summary.contains("ISTIO_MUTUAL"));
    assert_eq!(
        rule.badges
            .iter()
            .find(|badge| badge.label == "subsets")
            .and_then(|badge| badge.tooltip.as_deref()),
        Some("stable, canary")
    );
    let targets = topology
        .edges
        .iter()
        .find(|edge| {
            edge.kind == EdgeKind::Targets
                && edge.from.name == "order-processor"
                && edge.from.kind == "DestinationRule"
                && edge.to.kind == "Service"
        })
        .expect("destination rule target");
    assert_eq!(
        targets
            .metadata
            .get("mtls_mode")
            .and_then(|value| value.as_str()),
        Some("ISTIO_MUTUAL")
    );
    assert_eq!(
        targets.metadata.get("subsets"),
        Some(&json!(["stable", "canary"]))
    );
}
