#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

//! Shared helpers for the policy correlation integration tests.

use std::path::Path;
use std::sync::Arc;

use knect_core::correlate::policy::PolicyCorrelator;
use knect_core::correlate::policy::flagger::FlaggerPolicyCorrelator;
use knect_core::correlate::policy::istio::IstioPolicyCorrelator;
use knect_core::correlate::policy::kyverno::KyvernoPolicyCorrelator;
use knect_core::correlate::serving::ServingCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::store::ResourceStore;
use knect_proto::graph::{Edge, EdgeKind, Topology};
use knect_proto::{NodeCategory, ResourceId, ResourceNode};
use kube::api::DynamicObject;

pub fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

pub fn load_store() -> ResourceStore {
    let contents = std::fs::read_to_string(fixture_path("policy/policy.json"))
        .expect("policy fixture is readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("policy fixture is valid JSON");
    let mut store = ResourceStore::new();
    for object in objects {
        store.upsert(object);
    }
    store
}

pub fn topology(store: &ResourceStore, correlators: &[&dyn Correlator]) -> Topology {
    let ctx = CorrelationContext::new("fixture", 1, Arc::new(store.clone()));
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    for correlator in correlators {
        correlator
            .correlate(&ctx, &mut graph)
            .expect("correlation succeeds");
    }
    graph.build()
}

pub fn build_all(store: &ResourceStore) -> Topology {
    topology(store, &[&ServingCorrelator, &PolicyCorrelator::new()])
}

pub fn build_istio(store: &ResourceStore) -> Topology {
    topology(store, &[&ServingCorrelator, &IstioPolicyCorrelator])
}

pub fn build_flagger(store: &ResourceStore) -> Topology {
    topology(store, &[&ServingCorrelator, &FlaggerPolicyCorrelator])
}

pub fn build_istio_flagger(store: &ResourceStore) -> Topology {
    topology(
        store,
        &[
            &ServingCorrelator,
            &IstioPolicyCorrelator,
            &FlaggerPolicyCorrelator,
        ],
    )
}

pub fn build_kyverno(store: &ResourceStore) -> Topology {
    topology(store, &[&ServingCorrelator, &KyvernoPolicyCorrelator])
}

pub fn serving_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("serving.knative.dev/v1", kind, "knect-e2e", name).to_string()
}

pub fn istio_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("networking.istio.io/v1", kind, "knect-e2e", name).to_string()
}

pub fn security_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("security.istio.io/v1", kind, "knect-e2e", name).to_string()
}

pub fn flagger_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("flagger.app/v1beta1", kind, "knect-e2e", name).to_string()
}

pub fn kyverno_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("kyverno.io/v1", kind, "knect-e2e", name).to_string()
}

pub fn cluster_policy_id(name: &str) -> String {
    ResourceId::cluster_scoped("kyverno.io/v1", "ClusterPolicy", name).to_string()
}

pub fn apps_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("apps/v1", kind, "knect-e2e", name).to_string()
}

pub fn core_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("v1", kind, "knect-e2e", name).to_string()
}

pub fn triple(from: String, to: String, kind: EdgeKind) -> (String, String, EdgeKind) {
    (from, to, kind)
}

pub fn node_keys(topology: &Topology) -> Vec<(NodeCategory, String)> {
    let mut keys: Vec<(NodeCategory, String)> = topology
        .nodes
        .iter()
        .map(|node| (node.category, node.name.clone()))
        .collect();
    keys.sort();
    keys
}

pub fn policy_node_keys(topology: &Topology) -> Vec<(NodeCategory, String)> {
    let mut keys: Vec<(NodeCategory, String)> = topology
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.category,
                NodeCategory::Policy
                    | NodeCategory::Canary
                    | NodeCategory::VirtualService
                    | NodeCategory::DestinationRule
                    | NodeCategory::PeerAuthentication
                    | NodeCategory::AuthorizationPolicy
            )
        })
        .map(|node| (node.category, node.name.clone()))
        .collect();
    keys.sort();
    keys
}

pub fn edge_triples(topology: &Topology) -> Vec<(String, String, EdgeKind)> {
    let mut triples: Vec<(String, String, EdgeKind)> = topology
        .edges
        .iter()
        .map(|edge| (edge.from.to_string(), edge.to.to_string(), edge.kind))
        .collect();
    triples.sort();
    triples
}

pub fn node<'a>(topology: &'a Topology, category: NodeCategory, name: &str) -> &'a ResourceNode {
    topology
        .nodes
        .iter()
        .find(|node| node.category == category && node.name == name)
        .unwrap_or_else(|| panic!("missing node {category:?}/{name}"))
}

pub fn badge_labels(node: &ResourceNode) -> Vec<&str> {
    node.badges
        .iter()
        .map(|badge| badge.label.as_str())
        .collect()
}

pub fn edges<'a>(topology: &'a Topology, from: &str, to: &str, kind: EdgeKind) -> Vec<&'a Edge> {
    topology
        .edges
        .iter()
        .filter(|edge| edge.kind == kind && edge.from.name == from && edge.to.name == to)
        .collect()
}

pub fn edge<'a>(topology: &'a Topology, from: &str, to: &str, kind: EdgeKind) -> &'a Edge {
    let mut found = edges(topology, from, to, kind);
    found.sort_by(|left, right| left.id.cmp(&right.id));
    found
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("missing edge {from} -> {to} {kind:?}"))
}

pub fn serialized(topology: &Topology) -> String {
    serde_json::to_string(topology).expect("topology serializes")
}
