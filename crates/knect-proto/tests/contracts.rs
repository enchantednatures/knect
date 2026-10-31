#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use knect_proto::events::{SseEvent, SseEventKind};
use knect_proto::graph::{NodeCategory, ResourceNode, Topology};
use knect_proto::mutation::{MutationDiff, MutationSource};
use knect_proto::rpc::{GetTopologyParams, RpcRequest, RpcResponse};

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/contracts")
}

fn read_contract(name: &str) -> String {
    fs::read_to_string(contracts_dir().join(name)).unwrap()
}

fn assert_semantic_roundtrip<T>(value: &T, original: serde_json::Value)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let encoded = serde_json::to_value(value).unwrap();
    assert_eq!(encoded, original);
    let decoded: T = serde_json::from_value(encoded).unwrap();
    assert_eq!(&decoded, value);
}

#[test]
fn rpc_request_contract_roundtrips() {
    let original: serde_json::Value =
        serde_json::from_str(&read_contract("rpc_request.sample.json")).unwrap();
    let request: RpcRequest = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(request.jsonrpc, "2.0");
    assert_eq!(request.method, "graph.get_topology");
    assert_eq!(request.id, knect_proto::RpcId::Number(1));
    let params: GetTopologyParams =
        serde_json::from_value(request.params.clone().unwrap()).unwrap();
    assert_eq!(params.context.as_deref(), Some("kind-knect-e2e"));
    assert_eq!(params.min_generation, Some(42));
    assert_eq!(params.include.unwrap().len(), 4);
    assert_semantic_roundtrip(&request, original);
}

#[test]
fn rpc_error_contract_roundtrips() {
    let original: serde_json::Value =
        serde_json::from_str(&read_contract("rpc_error.sample.json")).unwrap();
    let response: RpcResponse = serde_json::from_value(original.clone()).unwrap();
    assert!(response.result.is_none());
    let error = response.error.as_ref().unwrap();
    assert_eq!(error.code, knect_proto::error::NOT_FOUND);
    let data = error.data.as_ref().unwrap();
    assert!(!data.retryable);
    assert_eq!(data.resource.as_ref().unwrap().name, "orders-v2");
    assert_semantic_roundtrip(&response, original);
}

#[test]
fn topology_contract_roundtrips() {
    let original: serde_json::Value =
        serde_json::from_str(&read_contract("topology.sample.json")).unwrap();
    let topology: Topology = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(topology.schema_version, knect_proto::SCHEMA_VERSION);
    assert_eq!(topology.generation, 42);
    assert_eq!(topology.nodes.len(), 4);
    assert_eq!(topology.edges.len(), 3);
    assert_eq!(topology.stats.node_count, 4);
    assert_eq!(topology.stats.edge_count, 3);
    assert_eq!(topology.stats.warning_count, 0);
    assert!(!topology.stats.truncated);
    assert!(has_category(&topology.nodes, NodeCategory::KService));
    assert!(has_category(&topology.nodes, NodeCategory::Revision));
    assert!(has_category(&topology.nodes, NodeCategory::Broker));
    assert!(has_category(&topology.nodes, NodeCategory::Trigger));
    assert_semantic_roundtrip(&topology, original);
}

#[test]
fn node_details_contract_roundtrips() {
    let original: serde_json::Value =
        serde_json::from_str(&read_contract("node_details.sample.json")).unwrap();
    let details: knect_proto::NodeDetails = serde_json::from_value(original.clone()).unwrap();
    assert!(details.yaml_redacted);
    assert!(details.yaml.is_some());
    assert!(!details.links.is_empty());
    assert!(!details.policies.is_empty());
    assert!(!details.storage.is_empty());
    assert!(!details.mutations.is_empty());
    assert!(details.metrics.is_some());
    assert_semantic_roundtrip(&details, original);
}

#[test]
fn mutation_diff_contract_roundtrips() {
    let original: serde_json::Value =
        serde_json::from_str(&read_contract("mutation_diff.sample.json")).unwrap();
    let diff: MutationDiff = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(diff.lines.len(), 3);
    assert_eq!(diff.lines[0].side, MutationSource::UserIntent);
    assert_eq!(diff.lines[1].side, MutationSource::KnativeDefaults);
    assert_eq!(diff.lines[2].side, MutationSource::PolicyMutators);
    assert!(diff.lines.iter().all(|line| line.owner.is_some()));
    assert_semantic_roundtrip(&diff, original);
}

#[test]
fn sse_patch_contract_roundtrips() {
    let frame = read_contract("sse_patch.sample.txt");
    let mut id = None;
    let mut event = None;
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("id: ") {
            id = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("event: ") {
            event = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data_lines.push(value.to_owned());
        }
    }
    assert_eq!(id.as_deref(), Some("42"));
    assert_eq!(event.as_deref(), Some("graph.patch"));
    assert!(!data_lines.is_empty());

    let data = data_lines.join("\n");
    let parsed: SseEvent = serde_json::from_str(&data).unwrap();
    assert_eq!(parsed.seq, 42);
    assert_eq!(parsed.event_type(), "graph.patch");
    assert!(matches!(parsed.kind, SseEventKind::GraphPatch { .. }));
    let original: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_semantic_roundtrip(&parsed, original);
}

fn has_category(nodes: &[ResourceNode], category: NodeCategory) -> bool {
    nodes.iter().any(|node| node.category == category)
}
