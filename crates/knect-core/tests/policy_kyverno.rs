#![allow(clippy::unwrap_used, clippy::expect_used)]

mod policy_support;

use knect_proto::NodeCategory;
use knect_proto::graph::{EdgeKind, Health, Severity};
use serde_json::json;

use policy_support::{
    build_all, build_kyverno, edges, load_store, node, policy_node_keys, serialized,
};

#[test]
fn kyverno_family_projects_policy_nodes() {
    let topology = build_kyverno(&load_store());
    assert_eq!(
        policy_node_keys(&topology),
        vec![
            (NodeCategory::Policy, "knect-e2e-mutate-labels".to_owned()),
            (NodeCategory::Policy, "knect-e2e-require-labels".to_owned()),
            (NodeCategory::Policy, "team-guard".to_owned()),
        ]
    );
    let require = node(&topology, NodeCategory::Policy, "knect-e2e-require-labels");
    assert_eq!(require.health, Health::Healthy);
    assert!(require.summary.contains("validate"));
    assert!(require.badges.iter().any(|badge| badge.label == "audit"));
    let mutate = node(&topology, NodeCategory::Policy, "knect-e2e-mutate-labels");
    assert!(mutate.summary.contains("mutate"));
    assert_eq!(
        mutate
            .annotations
            .get("knect.dev/policy-mode")
            .map(String::as_str),
        Some("mutate")
    );
}

#[test]
fn revision_report_maps_to_parent_knative_service() {
    let topology = build_kyverno(&load_store());
    let protected = edges(
        &topology,
        "order-processor",
        "knect-e2e-require-labels",
        EdgeKind::ProtectedBy,
    );
    assert!(protected.len() >= 2);
    let failing = protected
        .iter()
        .find(|edge| {
            edge.metadata.get("report").and_then(|value| value.as_str())
                == Some("order-processor-revision-report")
        })
        .expect("revision report edge");
    assert_eq!(
        failing
            .metadata
            .get("result")
            .and_then(|value| value.as_str()),
        Some("fail")
    );
    assert_eq!(
        failing
            .metadata
            .get("severity")
            .and_then(|value| value.as_str()),
        Some("high")
    );
    assert_eq!(
        failing
            .metadata
            .get("report_kind")
            .and_then(|value| value.as_str()),
        Some("PolicyReport")
    );
    let message = failing
        .metadata
        .get("message")
        .and_then(|value| value.as_str())
        .expect("message metadata");
    assert!(message.contains("<redacted>"));
    assert!(!message.contains("hunter2-sentinel"));

    let mutations = edges(
        &topology,
        "order-processor",
        "knect-e2e-mutate-labels",
        EdgeKind::MutatedBy,
    );
    assert!(
        mutations
            .iter()
            .any(|edge| edge.metadata.get("result") == Some(&json!("skip")))
    );
    assert!(
        mutations
            .iter()
            .any(|edge| edge.metadata.get("result") == Some(&json!("pass")))
    );

    let team = edges(
        &topology,
        "order-processor",
        "team-guard",
        EdgeKind::ProtectedBy,
    );
    assert!(
        team.iter()
            .any(|edge| edge.metadata.get("result") == Some(&json!("warn")))
    );
}

#[test]
fn pod_report_walks_the_owner_chain_to_the_service() {
    let topology = build_kyverno(&load_store());
    let pod_report = edges(
        &topology,
        "order-processor",
        "knect-e2e-require-labels",
        EdgeKind::ProtectedBy,
    )
    .into_iter()
    .find(|edge| {
        edge.metadata.get("report").and_then(|value| value.as_str())
            == Some("order-processor-pod-report")
    })
    .expect("pod report edge");
    assert_eq!(
        pod_report
            .metadata
            .get("result")
            .and_then(|value| value.as_str()),
        Some("pass")
    );
    assert_eq!(
        pod_report.from,
        knect_proto::ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            "knect-e2e",
            "order-processor"
        )
    );
}

#[test]
fn cluster_policy_report_maps_to_the_service() {
    let topology = build_kyverno(&load_store());
    let cluster = edges(
        &topology,
        "order-processor",
        "team-guard",
        EdgeKind::ProtectedBy,
    )
    .into_iter()
    .find(|edge| {
        edge.metadata
            .get("report_kind")
            .and_then(|value| value.as_str())
            == Some("ClusterPolicyReport")
    })
    .expect("cluster report edge");
    assert_eq!(
        cluster
            .metadata
            .get("report")
            .and_then(|value| value.as_str()),
        Some("knect-e2e-cluster-policy-report")
    );
    assert_eq!(
        cluster
            .metadata
            .get("result")
            .and_then(|value| value.as_str()),
        Some("pass")
    );
}

#[test]
fn policy_events_map_to_the_service_and_warn_on_violations() {
    let topology = build_kyverno(&load_store());
    let violation = edges(
        &topology,
        "order-processor",
        "knect-e2e-require-labels",
        EdgeKind::ProtectedBy,
    )
    .into_iter()
    .find(|edge| {
        edge.metadata.get("event").and_then(|value| value.as_str()) == Some("order-processor.1")
    })
    .expect("violation event edge");
    assert_eq!(
        violation
            .metadata
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("PolicyViolation")
    );
    assert_eq!(
        violation
            .metadata
            .get("severity")
            .and_then(|value| value.as_str()),
        Some("high")
    );
    assert_eq!(
        violation
            .metadata
            .get("reporting_component")
            .and_then(|value| value.as_str()),
        Some("kyverno")
    );
    assert_eq!(violation.metadata.get("count"), Some(&json!(3)));
    let message = violation
        .metadata
        .get("message")
        .and_then(|value| value.as_str())
        .expect("event message");
    assert!(message.contains("<redacted>"));
    assert!(!message.contains("event-sentinel"));

    let warning = topology
        .warnings
        .iter()
        .find(|warning| warning.code == "policy_violation")
        .expect("policy violation warning");
    assert_eq!(warning.severity, Severity::High);

    let applied = edges(
        &topology,
        "order-processor",
        "knect-e2e-mutate-labels",
        EdgeKind::MutatedBy,
    )
    .into_iter()
    .find(|edge| {
        edge.metadata.get("event").and_then(|value| value.as_str()) == Some("order-processor.2")
    })
    .expect("applied event edge");
    assert_eq!(
        applied
            .metadata
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("PolicyApplied")
    );
    assert_eq!(
        applied
            .metadata
            .get("result")
            .and_then(|value| value.as_str()),
        Some("pass")
    );
}

#[test]
fn policy_output_is_redacted_and_deterministic() {
    let store = load_store();
    let topology = build_all(&store);
    let output = serialized(&topology);
    assert!(!output.contains("hunter2-sentinel"));
    assert!(!output.contains("event-sentinel"));
    assert!(!output.contains("secretdata-sentinel"));
    assert!(output.contains("<redacted>"));
    assert_eq!(build_all(&store), build_all(&store));
}
