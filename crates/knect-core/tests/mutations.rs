#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Managed-fields parsing, YAML line mapping, and three-way attribution.

use std::path::Path;

use knect_core::mutations::{
    MutationDiffMode, MutationError, MutationSource, classify_manager, diff, records,
};
use knect_proto::mutation::ManagerKind;
use kube::api::DynamicObject;
use serde_json::json;

fn fixture(name: &str) -> DynamicObject {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures/mutations")
        .join(name);
    let raw = std::fs::read_to_string(path).expect("fixture is readable");
    serde_json::from_str(&raw).expect("fixture parses as a DynamicObject")
}

fn object_with(managed_fields: serde_json::Value, body: serde_json::Value) -> DynamicObject {
    let mut value = json!({
        "apiVersion": "serving.knative.dev/v1",
        "kind": "Service",
        "metadata": {
            "name": "hello",
            "namespace": "default",
            "managedFields": managed_fields
        }
    });
    if let (Some(root), Some(extra)) = (value.as_object_mut(), body.as_object()) {
        for (key, item) in extra {
            if key == "metadata"
                && let (Some(existing), Some(wanted)) = (
                    root.get_mut("metadata")
                        .and_then(serde_json::Value::as_object_mut),
                    item.as_object(),
                )
            {
                for (field, value) in wanted {
                    existing.insert(field.clone(), value.clone());
                }
                continue;
            }
            root.insert(key.clone(), item.clone());
        }
    }
    serde_json::from_value(value).expect("object parses")
}

#[test]
fn mutations_records_map_live_kservice_managers_and_paths() {
    let object = fixture("kservice-order-processor.json");
    let records = records(&object).expect("records are produced");
    assert_eq!(records.len(), 3);

    let controller = records
        .iter()
        .find(|record| record.manager == "controller")
        .expect("controller record");
    assert_eq!(controller.manager_kind, ManagerKind::Controller);
    assert_eq!(controller.subresource.as_deref(), Some("status"));
    assert!(
        controller
            .paths
            .iter()
            .any(|path| path.path.as_str() == "/status")
    );

    let kubectl = records
        .iter()
        .find(|record| record.manager == "kubectl-client-side-apply")
        .expect("kubectl record");
    assert_eq!(kubectl.manager_kind, ManagerKind::Kubectl);
    let containers = kubectl
        .paths
        .iter()
        .find(|path| path.path.as_str() == "/spec/template/spec/containers")
        .expect("whole containers list is owned");
    assert!(containers.yaml_line.is_some());
    assert!(containers.yaml_column.is_some());
    assert!(
        containers
            .value_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("nginxinc/nginx-unprivileged"))
    );
    assert!(
        kubectl
            .paths
            .iter()
            .any(|path| path.path.as_str() == "/spec/template/spec/timeoutSeconds")
    );
}

#[test]
fn mutations_three_way_attributes_kyverno_knative_and_user_lines() {
    let object = fixture("kservice-mutated.json");
    let mutation_diff = diff(&object, MutationDiffMode::ThreeWay).expect("diff is produced");

    let user = mutation_diff.user_intent.as_deref().expect("user intent");
    assert!(user.contains("image: nginxinc/nginx-unprivileged:1.27.4"));
    assert!(!user.contains("managedFields"));
    assert!(!user.contains("knect.dev/managed"));

    let defaults = mutation_diff
        .knative_defaults
        .as_deref()
        .expect("knative defaults");
    assert!(defaults.contains("idleTimeoutSeconds: 300"));
    assert!(defaults.contains("client.knative.dev/user-image"));

    let policy = mutation_diff
        .policy_mutators
        .as_deref()
        .expect("policy mutators");
    assert!(policy.contains("knect.dev/managed"));
    assert!(policy.contains("sidecar.istio.io/inject"));

    let managed_line = mutation_diff
        .lines
        .iter()
        .find(|line| line.text.contains("knect.dev/managed"))
        .expect("kyverno-owned line");
    assert_eq!(managed_line.side, MutationSource::PolicyMutators);
    assert_eq!(managed_line.owner.as_deref(), Some("kyverno"));
    assert_eq!(
        managed_line.path.as_ref().map(|path| path.as_str()),
        Some("/metadata/labels/knect.dev~1managed")
    );
    assert_eq!(managed_line.line_number, 9);

    let webhook_line = mutation_diff
        .lines
        .iter()
        .find(|line| line.text.contains("idleTimeoutSeconds"))
        .expect("webhook-owned line");
    assert_eq!(webhook_line.side, MutationSource::KnativeDefaults);
    assert_eq!(
        webhook_line.owner.as_deref(),
        Some("webhook.serving.knative.dev")
    );

    assert!(mutation_diff.lines.iter().any(|line| {
        line.side == MutationSource::UserIntent
            && line.owner.as_deref() == Some("kubectl-client-side-apply")
            && line.text.contains("image:")
    }));
    assert!(
        mutation_diff
            .lines
            .iter()
            .any(|line| line.side == MutationSource::PolicyMutators
                && line.owner.as_deref() == Some("istiod"))
    );
    assert!(
        mutation_diff
            .lines
            .iter()
            .any(|line| line.side == MutationSource::PolicyMutators
                && line.owner.as_deref() == Some("flagger"))
    );
}

#[test]
fn mutations_diff_is_deterministic_across_runs() {
    let object = fixture("kservice-mutated.json");
    let first = diff(&object, MutationDiffMode::ThreeWay).expect("first diff");
    let second = diff(&object, MutationDiffMode::ThreeWay).expect("second diff");
    assert_eq!(first, second);
    assert_eq!(first.lines, second.lines);

    let manager_first = diff(&object, MutationDiffMode::Managers).expect("managers diff");
    let manager_second = diff(&object, MutationDiffMode::Managers).expect("managers diff");
    assert_eq!(manager_first, manager_second);
}

#[test]
fn mutations_list_map_and_keyed_paths_resolve() {
    let object = object_with(
        json!([{
            "apiVersion": "serving.knative.dev/v1",
            "fieldsType": "FieldsV1",
            "fieldsV1": {
                "f:metadata": {
                    "f:annotations": {"f:example.com/key": {}}
                },
                "f:spec": {
                    "f:traffic": {"i:1": {"f:percent": {}}},
                    "f:containers": {
                        "k:{\"name\":\"sidecar\"}": {"f:image": {}}
                    }
                }
            },
            "manager": "kubectl-client-side-apply",
            "operation": "Update"
        }]),
        json!({
            "metadata": {"annotations": {"example.com/key": "value"}},
            "spec": {
                "traffic": [{"percent": 90}, {"percent": 10}],
                "containers": [{"name": "web", "image": "nginx"}, {"name": "sidecar", "image": "envoy"}]
            }
        }),
    );
    let parsed = records(&object).expect("records are produced");
    let paths: Vec<&str> = parsed[0]
        .paths
        .iter()
        .map(|path| path.path.as_str())
        .collect();
    assert!(paths.contains(&"/metadata/annotations/example.com~1key"));
    assert!(paths.contains(&"/spec/traffic/1/percent"));
    assert!(paths.contains(&"/spec/containers/1/image"));
    assert!(
        parsed[0]
            .paths
            .iter()
            .all(|path| path.yaml_line.is_some() && path.yaml_column.is_some())
    );
}

#[test]
fn mutations_malformed_fields_v1_is_a_typed_error() {
    let object = object_with(
        json!([{
            "fieldsType": "FieldsV1",
            "fieldsV1": {"f:spec": {"i:not-a-number": {}}},
            "manager": "kubectl-client-side-apply"
        }]),
        json!({"spec": {"containers": []}}),
    );
    let error = diff(&object, MutationDiffMode::ThreeWay).expect_err("malformed trie");
    assert!(matches!(
        error,
        MutationError::FieldsV1 { ref manager, .. } if manager == "kubectl-client-side-apply"
    ));

    let mut missing = object;
    missing.metadata.managed_fields = None;
    assert!(matches!(
        records(&missing),
        Err(MutationError::MissingManagedFields)
    ));
}

#[test]
fn mutations_managers_mode_groups_by_owner_source() {
    let object = fixture("kservice-mutated.json");
    let manager_diff = diff(&object, MutationDiffMode::Managers).expect("managers diff");
    assert!(manager_diff.user_intent.is_some());
    assert!(manager_diff.knative_defaults.is_none());
    assert!(manager_diff.policy_mutators.is_none());

    let kyverno = manager_diff
        .lines
        .iter()
        .find(|line| line.owner.as_deref() == Some("kyverno"))
        .expect("kyverno line");
    assert_eq!(kyverno.side, MutationSource::PolicyMutators);
    let kubectl = manager_diff
        .lines
        .iter()
        .find(|line| line.owner.as_deref() == Some("kubectl-client-side-apply"))
        .expect("kubectl line");
    assert_eq!(kubectl.side, MutationSource::UserIntent);
}

#[test]
fn mutations_single_document_does_not_fabricate_panes() {
    let object = object_with(
        json!([{
            "fieldsType": "FieldsV1",
            "fieldsV1": {"f:spec": {"f:template": {"f:spec": {"f:containers": {}}}}},
            "manager": "kubectl-client-side-apply",
            "operation": "Update"
        }]),
        json!({
            "metadata": {"labels": {"app": "hello"}},
            "spec": {"template": {"spec": {"containers": [{"image": "nginx"}]}}}
        }),
    );
    let mutation_diff = diff(&object, MutationDiffMode::ThreeWay).expect("diff is produced");
    assert!(mutation_diff.user_intent.is_some());
    assert!(mutation_diff.knative_defaults.is_none());
    assert!(mutation_diff.policy_mutators.is_none());
    assert!(
        mutation_diff
            .lines
            .iter()
            .any(|line| line.owner.as_deref() == Some("kubectl-client-side-apply"))
    );
}

#[test]
fn mutations_manager_classification_covers_the_fleet() {
    assert_eq!(
        classify_manager("webhook.serving.knative.dev").kind,
        ManagerKind::Knative
    );
    assert_eq!(classify_manager("kyverno").kind, ManagerKind::Kyverno);
    assert_eq!(classify_manager("istiod").kind, ManagerKind::Istio);
    assert_eq!(classify_manager("flagger").kind, ManagerKind::Flagger);
    assert_eq!(
        classify_manager("kubectl-client-side-apply").kind,
        ManagerKind::Kubectl
    );
    assert_eq!(classify_manager("controller").kind, ManagerKind::Controller);
}
