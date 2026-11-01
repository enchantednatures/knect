#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `mutations.get_diff` over the public dispatcher with an `AppState` store.

use std::path::Path;

use knect_core::config::Config;
use knect_proto::mutation::{MutationDiff, MutationSource};
use knect_proto::rpc::methods;
use knect_proto::{ResourceId, RpcRequest};
use knectd::{AppState, RpcDispatcher};
use kube::api::DynamicObject;
use serde_json::json;

fn fixture(name: &str) -> DynamicObject {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures/mutations")
        .join(name);
    let raw = std::fs::read_to_string(path).expect("fixture is readable");
    serde_json::from_str(&raw).expect("fixture parses")
}

fn id(name: &str) -> ResourceId {
    ResourceId::namespaced("serving.knative.dev/v1", "Service", "knect-e2e", name)
}

async fn state_with(object: DynamicObject) -> AppState {
    let state = AppState::new(Config::default(), true);
    state.store.write().await.upsert(object);
    state
}

#[tokio::test]
async fn mutations_rpc_returns_three_way_diff_from_the_store() {
    let state = state_with(fixture("kservice-mutated.json")).await;
    let dispatcher = RpcDispatcher::new(state);
    let response = dispatcher
        .dispatch_async(RpcRequest::new(
            1_i64,
            methods::MUTATIONS_GET_DIFF,
            Some(json!({"id": id("order-processor-mutated")})),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let diff: MutationDiff =
        serde_json::from_value(response.result.expect("result")).expect("diff decodes");
    assert_eq!(diff.resource, id("order-processor-mutated"));
    assert!(diff.user_intent.is_some());
    assert!(diff.knative_defaults.is_some());
    assert!(diff.policy_mutators.is_some());
    let kyverno = diff
        .lines
        .iter()
        .find(|line| {
            line.owner.as_deref() == Some("kyverno") && line.text.contains("knect.dev/managed")
        })
        .expect("kyverno managed line");
    assert_eq!(kyverno.side, MutationSource::PolicyMutators);
    assert_eq!(kyverno.line_number, 9);
    assert!(kyverno.text.contains("knect.dev/managed"));
}

#[tokio::test]
async fn mutations_rpc_is_deterministic() {
    let state = state_with(fixture("kservice-mutated.json")).await;
    let dispatcher = RpcDispatcher::new(state);
    let request = || {
        RpcRequest::new(
            1_i64,
            methods::MUTATIONS_GET_DIFF,
            Some(json!({"id": id("order-processor-mutated")})),
        )
    };
    let first = dispatcher
        .dispatch_async(request())
        .await
        .expect("first response");
    let second = dispatcher
        .dispatch_async(request())
        .await
        .expect("second response");
    assert_eq!(first.result, second.result);
}

#[tokio::test]
async fn mutations_rpc_returns_not_found_for_absent_resources() {
    let state = AppState::new(Config::default(), true);
    let dispatcher = RpcDispatcher::new(state);
    let response = dispatcher
        .dispatch_async(RpcRequest::new(
            1_i64,
            methods::MUTATIONS_GET_DIFF,
            Some(json!({"id": id("missing")})),
        ))
        .await
        .expect("response");
    let error = response.error.expect("error");
    assert_eq!(error.code, knect_proto::error::NOT_FOUND);
    assert!(error.message.contains("missing"));
}

#[tokio::test]
async fn mutations_rpc_requires_managed_fields() {
    let mut object = fixture("kservice-order-processor.json");
    object.metadata.name = Some("without-managed-fields".to_owned());
    object.metadata.managed_fields = None;
    let state = state_with(object).await;
    let dispatcher = RpcDispatcher::new(state);
    let response = dispatcher
        .dispatch_async(RpcRequest::new(
            1_i64,
            methods::MUTATIONS_GET_DIFF,
            Some(json!({"id": id("without-managed-fields")})),
        ))
        .await
        .expect("response");
    let error = response.error.expect("error");
    assert_eq!(error.code, knect_proto::error::NOT_FOUND);
    assert!(error.message.contains("managedFields"));
}
