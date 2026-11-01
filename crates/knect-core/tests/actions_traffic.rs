#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Traffic-split validation, ownership, and merge-patch tests.

mod actions_support;

use actions_support::{NS, SERVICE, actions, cluster, revision, service, split, traffic_params};
use knect_core::ActionExecutor;
use knect_core::kube::FakeReader;
use knect_proto::error::{INVALID_PARAMS, KUBE_UNAVAILABLE};
use knect_proto::rpc::SetTrafficSplitParams;
use knect_proto::{Gvk, ResourceId};
use serde_json::json;

#[tokio::test]
async fn actions_traffic_split_patches_spec_traffic_in_input_order() {
    let reader = cluster().with_patch_response(Ok(service()));
    let result = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![
                split("hello-00002", 90, Some("stable")),
                split("hello-00001", 10, None),
            ],
            None,
        ))
        .await
        .expect("traffic split succeeds");

    assert_eq!(result.action, "actions.set_traffic_split");
    assert!(result.success);
    assert!(!result.dry_run);
    assert_eq!(
        result.resource,
        Some(ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            NS,
            SERVICE
        ))
    );

    let patches = reader.patch_calls();
    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0].gvk,
        Gvk::new("serving.knative.dev", "v1", "Service")
    );
    assert_eq!(patches[0].namespace, NS);
    assert_eq!(patches[0].name, SERVICE);
    assert!(!patches[0].dry_run);
    assert_eq!(
        patches[0].patch,
        json!({
            "spec": {
                "traffic": [
                    {"revisionName": "hello-00002", "percent": 90, "tag": "stable"},
                    {"revisionName": "hello-00001", "percent": 10}
                ]
            }
        })
    );
}

#[tokio::test]
async fn actions_traffic_split_allows_zero_and_hundred_percent() {
    let reader = cluster().with_patch_response(Ok(service()));
    let result = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![
                split("hello-00001", 100, None),
                split("hello-00002", 0, None),
            ],
            Some(false),
        ))
        .await
        .expect("zero share is legal");
    assert!(result.success);
    assert_eq!(
        reader.patch_calls()[0].patch,
        json!({
            "spec": {
                "traffic": [
                    {"revisionName": "hello-00001", "percent": 100},
                    {"revisionName": "hello-00002", "percent": 0}
                ]
            }
        })
    );
}

#[tokio::test]
async fn actions_traffic_split_dry_run_marks_the_patch_as_dry_run() {
    let reader = cluster().with_patch_response(Ok(service()));
    let result = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![split("hello-00001", 100, None)],
            Some(true),
        ))
        .await
        .expect("dry run succeeds");

    assert!(result.success);
    assert!(result.dry_run);
    assert!(result.message.starts_with("dry run: "));
    let patches = reader.patch_calls();
    assert_eq!(patches.len(), 1);
    assert!(patches[0].dry_run);
}

#[tokio::test]
async fn actions_traffic_split_rejects_empty_splits_before_any_api_call() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(Vec::new(), None))
        .await
        .expect_err("empty split is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(reader.get_calls().is_empty());
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_rejects_empty_identity() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(SetTrafficSplitParams {
            namespace: String::new(),
            service: SERVICE.to_owned(),
            splits: vec![split("hello-00001", 100, None)],
            dry_run: None,
        })
        .await
        .expect_err("empty namespace is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);

    let error = actions(&reader)
        .set_traffic_split(SetTrafficSplitParams {
            namespace: NS.to_owned(),
            service: String::new(),
            splits: vec![split("hello-00001", 100, None)],
            dry_run: None,
        })
        .await
        .expect_err("empty service is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_rejects_percent_sum_that_is_not_one_hundred() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![
                split("hello-00001", 80, None),
                split("hello-00002", 10, None),
            ],
            None,
        ))
        .await
        .expect_err("sum must be 100");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(error.to_string().contains("sum to 100"));
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_rejects_percent_over_one_hundred() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![
                split("hello-00001", 100, None),
                split("hello-00002", 101, None),
            ],
            None,
        ))
        .await
        .expect_err("percent over 100 is invalid");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(error.to_string().contains("exceeds 100"));
}

#[tokio::test]
async fn actions_traffic_split_rejects_duplicate_revisions() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(
            vec![
                split("hello-00001", 50, None),
                split("hello-00001", 50, None),
            ],
            None,
        ))
        .await
        .expect_err("duplicates are invalid");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(error.to_string().contains("more than once"));
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_rejects_revisions_owned_by_another_service() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(vec![split("other-00001", 100, None)], None))
        .await
        .expect_err("foreign revision is rejected");
    assert_eq!(error.code(), INVALID_PARAMS);
    assert!(error.to_string().contains("not owned"));
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_propagates_missing_revision_errors() {
    let reader = cluster();
    let error = actions(&reader)
        .set_traffic_split(traffic_params(vec![split("hello-99999", 100, None)], None))
        .await
        .expect_err("missing revision is an API error");
    assert_eq!(error.code(), KUBE_UNAVAILABLE);
    assert!(reader.patch_calls().is_empty());
}

#[tokio::test]
async fn actions_traffic_split_only_requires_owned_revisions() {
    let reader = FakeReader::new()
        .with_object(revision("hello-00001"))
        .with_patch_response(Ok(service()));
    let result = actions(&reader)
        .set_traffic_split(traffic_params(vec![split("hello-00001", 100, None)], None))
        .await
        .expect("revision ownership is enough to validate");
    assert!(result.success);
    assert_eq!(reader.patch_calls().len(), 1);
}
