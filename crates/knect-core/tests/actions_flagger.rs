#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Flagger Canary promote and abort patch tests.

mod actions_support;

use actions_support::{NS, SERVICE, actions, canary, flagger_params};
use knect_core::ActionExecutor;
use knect_core::error::CoreError;
use knect_core::kube::FakeReader;
use knect_proto::error::{INVALID_PARAMS, KUBE_UNAVAILABLE};
use knect_proto::rpc::{FlaggerAction, FlaggerControlParams};
use knect_proto::{Gvk, ResourceId};
use kube::core::Status;
use serde_json::json;

#[tokio::test]
async fn actions_flagger_promote_sets_skip_analysis() {
    let reader = FakeReader::new().with_patch_response(Ok(canary()));
    let result = actions(&reader)
        .flagger_control(flagger_params(FlaggerAction::Promote, None))
        .await
        .expect("promote succeeds");

    assert_eq!(result.action, "actions.flagger_control");
    assert!(result.success);
    assert!(!result.dry_run);
    assert_eq!(
        result.resource,
        Some(ResourceId::namespaced(
            "flagger.app/v1beta1",
            "Canary",
            NS,
            SERVICE
        ))
    );
    let patches = reader.patch_calls();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].gvk, Gvk::new("flagger.app", "v1beta1", "Canary"));
    assert!(!patches[0].dry_run);
    assert_eq!(patches[0].patch, json!({"spec": {"skipAnalysis": true}}));
    assert!(result.message.contains("promoted"));
}

#[tokio::test]
async fn actions_flagger_abort_suspends_and_clears_skip_analysis() {
    let reader = FakeReader::new().with_patch_response(Ok(canary()));
    let result = actions(&reader)
        .flagger_control(flagger_params(FlaggerAction::Abort, Some(true)))
        .await
        .expect("abort succeeds");

    assert!(result.dry_run);
    assert!(result.message.starts_with("dry run: "));
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
async fn actions_flagger_rejects_empty_identity_before_writing() {
    let reader = FakeReader::new();
    let error = actions(&reader)
        .flagger_control(FlaggerControlParams {
            namespace: String::new(),
            canary: SERVICE.to_owned(),
            action: FlaggerAction::Promote,
            dry_run: None,
        })
        .await
        .expect_err("empty namespace is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);

    let error = actions(&reader)
        .flagger_control(FlaggerControlParams {
            namespace: NS.to_owned(),
            canary: String::new(),
            action: FlaggerAction::Promote,
            dry_run: None,
        })
        .await
        .expect_err("empty canary is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_flagger_propagates_patch_failures() {
    let conflict = CoreError::Kube(kube::Error::Api(
        Status {
            code: 409,
            reason: "Conflict".to_owned(),
            message: "canary is progressing".to_owned(),
            ..Status::default()
        }
        .boxed(),
    ));
    let reader = FakeReader::new().with_patch_response(Err(conflict));
    let error = actions(&reader)
        .flagger_control(flagger_params(FlaggerAction::Promote, None))
        .await
        .expect_err("patch failure propagates");
    assert_eq!(error.code(), KUBE_UNAVAILABLE);
}
