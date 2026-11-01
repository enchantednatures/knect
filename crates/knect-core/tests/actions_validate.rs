#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Server-side dry-run manifest validation tests.

mod actions_support;

use actions_support::{NS, SERVICE, actions, manifest, mutated_service, object};
use knect_core::ActionExecutor;
use knect_core::error::CoreError;
use knect_core::kube::{DryRunResult, FakeReader};
use knect_proto::ResourceId;
use knect_proto::error::INVALID_PARAMS;
use knect_proto::policy::{PolicyKind, PolicyResult};
use knect_proto::rpc::ValidateManifestParams;
use kube::api::TypeMeta;
use kube::core::Status;
use kube::core::response::{StatusCause, StatusDetails};
use serde_json::json;

#[tokio::test]
async fn actions_validate_manifest_returns_server_mutations_and_warnings() {
    let reader = FakeReader::new().with_dry_run_response(Ok(DryRunResult {
        object: mutated_service(),
        warnings: vec!["defaulted spec.traffic".to_owned()],
    }));
    let result = actions(&reader)
        .validate_manifest(manifest(
            "apiVersion: serving.knative.dev/v1\nkind: Service\nmetadata:\n  name: hello\n  namespace: default\nspec: {}\n",
        ))
        .await
        .expect("dry-run succeeds");

    assert!(result.valid);
    let mutated = result.mutated_manifest.expect("mutated manifest");
    assert!(mutated.contains("latestRevision"));
    assert!(mutated.contains("served"));
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0].code, "admission_warning");
    assert_eq!(result.warnings[0].message, "defaulted spec.traffic");
    assert_eq!(
        result.warnings[0].resource,
        Some(ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            NS,
            SERVICE
        ))
    );

    let calls = reader.dry_run_calls();
    assert_eq!(calls.len(), 1);
    assert!(reader.patch_calls().is_empty());
    assert!(reader.get_calls().is_empty());
}

#[tokio::test]
async fn actions_validate_manifest_applies_default_kind_and_namespace() {
    let mut mutated = object("v1", "ConfigMap", "ops", "settings");
    mutated.data = json!({"data": {"key": "value"}});
    let reader = FakeReader::new().with_dry_run_response(Ok(DryRunResult {
        object: mutated,
        warnings: Vec::new(),
    }));
    let result = actions(&reader)
        .validate_manifest(ValidateManifestParams {
            manifest: "apiVersion: v1\nmetadata:\n  name: settings\n".to_owned(),
            namespace: Some("ops".to_owned()),
            default_kind: Some("ConfigMap".to_owned()),
        })
        .await
        .expect("defaults complete the identity");

    assert!(result.valid);
    let calls = reader.dry_run_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].types,
        Some(TypeMeta {
            api_version: "v1".to_owned(),
            kind: "ConfigMap".to_owned(),
        })
    );
    assert_eq!(calls[0].metadata.namespace.as_deref(), Some("ops"));
}

#[tokio::test]
async fn actions_validate_manifest_keeps_an_explicit_namespace() {
    let reader = FakeReader::new().with_dry_run_response(Ok(DryRunResult {
        object: object("v1", "ConfigMap", "manifest-ns", "settings"),
        warnings: Vec::new(),
    }));
    let result = actions(&reader)
        .validate_manifest(ValidateManifestParams {
            manifest:
                "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n  namespace: manifest-ns\n"
                    .to_owned(),
            namespace: Some("other".to_owned()),
            default_kind: Some("Service".to_owned()),
        })
        .await
        .expect("explicit namespace wins");
    assert!(result.valid);
    assert_eq!(
        reader.dry_run_calls()[0].metadata.namespace.as_deref(),
        Some("manifest-ns")
    );
}

#[tokio::test]
async fn actions_validate_manifest_rejects_missing_identity_fields() {
    let reader = FakeReader::new();
    let cases = [
        (manifest(""), "not be empty"),
        (
            manifest("apiVersion: v1\nkind: ConfigMap\n"),
            "metadata.name",
        ),
        (
            manifest("apiVersion: v1\nmetadata:\n  name: settings\n"),
            "kind or supply default_kind",
        ),
        (
            manifest("kind: ConfigMap\nmetadata:\n  name: settings\n"),
            "apiVersion",
        ),
        (manifest("apiVersion: [\n"), "not valid YAML"),
    ];
    for (params, expected) in cases {
        let error = actions(&reader)
            .validate_manifest(params)
            .await
            .expect_err("invalid manifest");
        assert_eq!(error.code(), INVALID_PARAMS);
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}' in '{error}'"
        );
    }
    assert!(reader.dry_run_calls().is_empty());
}

#[tokio::test]
async fn actions_validate_manifest_surfaces_kyverno_admission_denials() {
    let reader = FakeReader::new().with_dry_run_response(Err(CoreError::Kube(
        kube::Error::Api(
            Status {
                code: 422,
                reason: "Invalid".to_owned(),
                message:
                    "admission webhook \"validate.kyverno.svc\" denied the request: policy require-labels failed"
                        .to_owned(),
                ..Status::default()
            }
            .boxed(),
        ),
    )));
    let result = actions(&reader)
        .validate_manifest(ValidateManifestParams {
            manifest: "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n".to_owned(),
            namespace: Some(NS.to_owned()),
            default_kind: None,
        })
        .await
        .expect("admission denial is a validation result");

    assert!(!result.valid);
    assert!(result.mutated_manifest.is_none());
    assert_eq!(result.violations.len(), 1);
    let violation = &result.violations[0];
    assert_eq!(violation.kind, PolicyKind::Kyverno);
    assert_eq!(violation.result, PolicyResult::Fail);
    assert_eq!(
        violation.resource,
        Some(ResourceId::namespaced("v1", "ConfigMap", NS, "settings"))
    );
    assert!(violation.message.contains("require-labels"));
    assert_eq!(
        violation.details.get("code").map(String::as_str),
        Some("422")
    );
}

#[tokio::test]
async fn actions_validate_manifest_expands_admission_causes() {
    let reader = FakeReader::new().with_dry_run_response(Err(CoreError::Kube(kube::Error::Api(
        Status {
            code: 403,
            reason: "Forbidden".to_owned(),
            message: "admission denied".to_owned(),
            details: Some(StatusDetails {
                name: String::new(),
                group: String::new(),
                kind: String::new(),
                uid: String::new(),
                causes: vec![StatusCause {
                    reason: "PolicyViolation".to_owned(),
                    message: "require-labels: missing app label".to_owned(),
                    field: "metadata.labels".to_owned(),
                }],
                retry_after_seconds: 0,
            }),
            ..Status::default()
        }
        .boxed(),
    ))));
    let result = actions(&reader)
        .validate_manifest(manifest(
            "apiVersion: v1\nkind: Pod\nmetadata:\n  name: web\n",
        ))
        .await
        .expect("admission causes are surfaced");

    assert!(!result.valid);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(
        result.violations[0]
            .details
            .get("field")
            .map(String::as_str),
        Some("metadata.labels")
    );
    assert!(result.violations[0].message.contains("require-labels"));
}

#[tokio::test]
async fn actions_validate_manifest_surfaces_policy_errors() {
    let reader = FakeReader::new().with_dry_run_response(Err(CoreError::Policy(
        "kyverno policy require-labels denied the request".to_owned(),
    )));
    let result = actions(&reader)
        .validate_manifest(manifest(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n",
        ))
        .await
        .expect("policy error is a validation result");

    assert!(!result.valid);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].kind, PolicyKind::Kyverno);
    assert_eq!(result.violations[0].result, PolicyResult::Fail);
    assert!(result.violations[0].message.contains("require-labels"));
}

#[tokio::test]
async fn actions_validate_manifest_propagates_transport_failures() {
    let reader = FakeReader::new()
        .with_dry_run_response(Err(CoreError::Internal("connection reset".to_owned())));
    let error = actions(&reader)
        .validate_manifest(manifest(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n",
        ))
        .await
        .expect_err("transport failure propagates");
    assert_eq!(error.to_string(), "internal error: connection reset");
}
