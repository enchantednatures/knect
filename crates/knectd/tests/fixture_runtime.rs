#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Public-surface proof that fixture startup serves the real correlation graph.

use std::path::{Path, PathBuf};

use knect_core::config::Config;
use knect_proto::NodeCategory;
use knect_proto::rpc::methods;
use knectd::app::AppState;
use knectd::fixture::FixtureMode;
use knectd::rpc::RpcDispatcher;
use knectd::runtime::Runtime;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fixtures")
}

#[tokio::test]
async fn fixture_runtime_serves_serving_and_eventing_over_rpc() {
    let state = AppState::new(Config::default(), true);
    let cancel = CancellationToken::new();
    let runtime = Runtime::new(
        state.clone(),
        FixtureMode::Enabled(Some(fixture_dir())),
        cancel.clone(),
    );
    runtime.start().await.expect("fixture runtime starts");

    let topology = state.topology();
    for category in [
        NodeCategory::KService,
        NodeCategory::Revision,
        NodeCategory::Broker,
        NodeCategory::Trigger,
    ] {
        assert!(
            topology.nodes.iter().any(|node| node.category == category),
            "missing {category:?}"
        );
    }
    assert!(!topology.edges.is_empty());
    assert!(topology.generation >= 1);

    let service = topology
        .nodes
        .iter()
        .find(|node| node.category == NodeCategory::KService)
        .expect("KService node");
    let dispatcher = RpcDispatcher::new(state.clone());
    let response = dispatcher
        .dispatch_async(knect_proto::RpcRequest::new(
            1_i64,
            methods::GRAPH_GET_NODE_DETAILS,
            Some(json!({"id": service.id, "include_yaml": true})),
        ))
        .await
        .expect("response");
    assert!(response.error.is_none());
    let details: knect_proto::graph::NodeDetails =
        serde_json::from_value(response.result.expect("result")).expect("details decode");
    assert!(details.yaml_redacted);
    assert!(
        details
            .yaml
            .as_deref()
            .is_some_and(|yaml| yaml.contains("Service"))
    );
    assert!(!details.policies.is_empty());

    cancel.cancel();
    runtime.run().await.expect("runtime stops");
}
