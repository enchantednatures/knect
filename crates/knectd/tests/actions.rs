#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Dispatcher-level proof of the action gates and the real executor wiring.

use std::sync::Arc;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use knect_core::actions::KubeActions;
use knect_core::kube::{DryRunResult, FakeReader};
use knect_core::{config::Config, error::CoreError};
use knect_proto::error::{ACTIONS_DISABLED, INVALID_PARAMS};
use knect_proto::events::ActionResult;
use knect_proto::policy::ValidationResult;
use knect_proto::{RpcRequest, RpcResponse, rpc::methods};
use knectd::{AppState, RpcDispatcher};
use kube::api::{DynamicObject, TypeMeta};
use kube::core::Status;
use serde_json::json;

const NS: &str = "default";
const SERVICE: &str = "hello";

fn config(actions_enabled: bool, allow_actions: bool) -> Config {
    let mut config = Config::default();
    config.actions.enabled = actions_enabled;
    config.daemon.allow_actions = allow_actions;
    config
}

fn object(api_version: &str, kind: &str, namespace: &str, name: &str) -> DynamicObject {
    DynamicObject {
        types: Some(TypeMeta {
            api_version: api_version.to_owned(),
            kind: kind.to_owned(),
        }),
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            uid: Some(format!("uid-{name}")),
            ..ObjectMeta::default()
        },
        data: json!({}),
    }
}

fn revision(name: &str) -> DynamicObject {
    let mut object = object("serving.knative.dev/v1", "Revision", NS, name);
    object.metadata.owner_references = Some(vec![OwnerReference {
        api_version: "serving.knative.dev/v1".to_owned(),
        kind: "Configuration".to_owned(),
        name: SERVICE.to_owned(),
        uid: "uid-config-hello".to_owned(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }]);
    object
}

fn cluster() -> FakeReader {
    FakeReader::new()
        .with_object(object("serving.knative.dev/v1", "Service", NS, SERVICE))
        .with_object(revision("hello-00001"))
        .with_object(revision("hello-00002"))
}

fn enabled_state(reader: &FakeReader) -> AppState {
    AppState::new(config(true, true), true)
        .with_actions(Arc::new(KubeActions::new(Arc::new(reader.clone()))))
}

fn request(method: &str, params: serde_json::Value) -> RpcRequest {
    RpcRequest::new(1_i64, method, Some(params))
}

fn error_code(response: RpcResponse) -> i32 {
    response.error.expect("error response").code
}

fn traffic_request(percent_second: u8) -> serde_json::Value {
    json!({
        "namespace": NS,
        "service": SERVICE,
        "splits": [
            {"revision": "hello-00001", "percent": 100 - percent_second},
            {"revision": "hello-00002", "percent": percent_second}
        ]
    })
}

#[tokio::test]
async fn actions_rpc_stays_disabled_until_both_gates_are_true() {
    for (actions_enabled, allow_actions) in [(false, false), (true, false), (false, true)] {
        let dispatcher =
            RpcDispatcher::new(AppState::new(config(actions_enabled, allow_actions), true));
        let response = dispatcher
            .dispatch_async(request(
                methods::ACTIONS_SET_TRAFFIC_SPLIT,
                traffic_request(0),
            ))
            .await
            .expect("response");
        assert_eq!(error_code(response), ACTIONS_DISABLED);

        let response = dispatcher
            .dispatch_async(request(
                methods::ACTIONS_FLAGGER_CONTROL,
                json!({"namespace": NS, "canary": SERVICE, "action": "promote"}),
            ))
            .await
            .expect("response");
        assert_eq!(error_code(response), ACTIONS_DISABLED);
    }
}

#[tokio::test]
async fn actions_rpc_set_traffic_split_calls_the_executor_when_both_gates_are_true() {
    let reader =
        cluster().with_patch_response(Ok(object("serving.knative.dev/v1", "Service", NS, SERVICE)));
    let dispatcher = RpcDispatcher::new(enabled_state(&reader));
    let response = dispatcher
        .dispatch_async(request(
            methods::ACTIONS_SET_TRAFFIC_SPLIT,
            traffic_request(10),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let result: ActionResult =
        serde_json::from_value(response.result.expect("result")).expect("result decodes");
    assert_eq!(result.action, "actions.set_traffic_split");
    assert!(result.success);

    let patches = reader.patch_calls();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].name, SERVICE);
    assert_eq!(
        patches[0].patch,
        json!({
            "spec": {
                "traffic": [
                    {"revisionName": "hello-00001", "percent": 90},
                    {"revisionName": "hello-00002", "percent": 10}
                ]
            }
        })
    );
}

#[tokio::test]
async fn actions_rpc_maps_invalid_traffic_splits_to_invalid_params() {
    let reader = cluster();
    let dispatcher = RpcDispatcher::new(enabled_state(&reader));
    let response = dispatcher
        .dispatch_async(request(
            methods::ACTIONS_SET_TRAFFIC_SPLIT,
            json!({
                "namespace": NS,
                "service": SERVICE,
                "splits": [{"revision": "hello-00001", "percent": 90}]
            }),
        ))
        .await
        .expect("response");
    let error = response.error.expect("validation error");
    assert_eq!(error.code, INVALID_PARAMS);
    assert!(error.message.contains("sum to 100"));
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_rpc_flagger_control_calls_the_executor_when_both_gates_are_true() {
    let reader = FakeReader::new().with_patch_response(Ok(object(
        "flagger.app/v1beta1",
        "Canary",
        NS,
        SERVICE,
    )));
    let dispatcher = RpcDispatcher::new(enabled_state(&reader));
    let response = dispatcher
        .dispatch_async(request(
            methods::ACTIONS_FLAGGER_CONTROL,
            json!({"namespace": NS, "canary": SERVICE, "action": "abort", "dry_run": true}),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let result: ActionResult =
        serde_json::from_value(response.result.expect("result")).expect("result decodes");
    assert!(result.dry_run);
    assert!(result.message.contains("aborted"));

    let patches = reader.patch_calls();
    assert_eq!(patches.len(), 1);
    assert!(patches[0].dry_run);
    assert_eq!(
        patches[0].patch,
        json!({"spec": {"suspend": true, "skipAnalysis": false}})
    );
}

#[tokio::test]
async fn actions_rpc_validate_manifest_returns_dry_run_mutations() {
    let mut mutated = object("serving.knative.dev/v1", "Service", NS, SERVICE);
    mutated.data = json!({"spec": {"traffic": [{"latestRevision": true, "percent": 100}]}});
    let reader = FakeReader::new().with_dry_run_response(Ok(DryRunResult {
        object: mutated,
        warnings: vec!["defaulted spec.traffic".to_owned()],
    }));
    let dispatcher = RpcDispatcher::new(enabled_state(&reader));
    let response = dispatcher
        .dispatch_async(request(
            methods::POLICY_VALIDATE_MANIFEST,
            json!({
                "manifest": "apiVersion: serving.knative.dev/v1\nkind: Service\nmetadata:\n  name: hello\nspec: {}\n",
                "namespace": NS
            }),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let result: ValidationResult =
        serde_json::from_value(response.result.expect("result")).expect("result decodes");
    assert!(result.valid);
    assert!(
        result
            .mutated_manifest
            .as_deref()
            .is_some_and(|yaml| yaml.contains("latestRevision"))
    );
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(reader.dry_run_calls().len(), 1);
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_rpc_validate_manifest_surfaces_admission_denials_as_violations() {
    let reader = FakeReader::new().with_dry_run_response(Err(CoreError::Kube(kube::Error::Api(
        Status {
            code: 403,
            reason: "Forbidden".to_owned(),
            message: "kyverno policy require-labels denied the request".to_owned(),
            ..Status::default()
        }
        .boxed(),
    ))));
    let dispatcher = RpcDispatcher::new(enabled_state(&reader));
    let response = dispatcher
        .dispatch_async(request(
            methods::POLICY_VALIDATE_MANIFEST,
            json!({
                "manifest": "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n",
                "namespace": NS
            }),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let result: ValidationResult =
        serde_json::from_value(response.result.expect("result")).expect("result decodes");
    assert!(!result.valid);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(
        result.violations[0].kind,
        knect_proto::policy::PolicyKind::Kyverno
    );
    assert!(result.violations[0].message.contains("require-labels"));
}

#[tokio::test]
async fn actions_rpc_validate_manifest_maps_malformed_yaml_to_invalid_params() {
    let dispatcher = RpcDispatcher::new(enabled_state(&FakeReader::new()));
    let response = dispatcher
        .dispatch_async(request(
            methods::POLICY_VALIDATE_MANIFEST,
            json!({"manifest": "apiVersion: [\n"}),
        ))
        .await
        .expect("response");
    assert_eq!(error_code(response), INVALID_PARAMS);
}
