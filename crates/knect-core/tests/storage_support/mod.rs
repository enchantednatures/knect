#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

//! Shared helpers for the storage correlation test binaries.

use std::path::Path;
use std::sync::Arc;

use knect_core::correlate::serving::ServingCorrelator;
use knect_core::correlate::storage::StorageCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::store::ResourceStore;
use knect_proto::graph::{EdgeKind, Topology};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Value, json};

pub(crate) const NAMESPACE: &str = "knect-e2e";

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

pub(crate) fn load_store() -> ResourceStore {
    let contents = std::fs::read_to_string(fixture_path("storage/storage.json"))
        .expect("storage fixture is readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("storage fixture is valid JSON");
    let mut store = ResourceStore::new();
    for object in objects {
        store.upsert(object);
    }
    store
}

pub(crate) fn build_topology(store: &ResourceStore) -> Topology {
    let ctx = CorrelationContext::new("fixture", 1, Arc::new(store.clone()));
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    ServingCorrelator
        .correlate(&ctx, &mut graph)
        .expect("serving correlation succeeds");
    StorageCorrelator
        .correlate(&ctx, &mut graph)
        .expect("storage correlation succeeds");
    graph.build()
}

pub(crate) fn serving_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("serving.knative.dev/v1", kind, NAMESPACE, name).to_string()
}

pub(crate) fn apps_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("apps/v1", kind, NAMESPACE, name).to_string()
}

pub(crate) fn core_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("v1", kind, NAMESPACE, name).to_string()
}

pub(crate) fn cluster_id(api_version: &str, kind: &str, name: &str) -> String {
    ResourceId::cluster_scoped(api_version, kind, name).to_string()
}

pub(crate) fn ceph_id(kind: &str, name: &str) -> String {
    ResourceId::namespaced("ceph.rook.io/v1", kind, "rook-ceph", name).to_string()
}

pub(crate) fn storage_edges(topology: &Topology) -> Vec<(String, String, EdgeKind)> {
    let mut edges: Vec<(String, String, EdgeKind)> = topology
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.kind,
                EdgeKind::Mounts | EdgeKind::ProvisionedBy | EdgeKind::BackedBy
            )
        })
        .map(|edge| (edge.from.to_string(), edge.to.to_string(), edge.kind))
        .collect();
    edges.sort();
    edges
}

pub(crate) fn edge<'a>(
    topology: &'a Topology,
    from: &str,
    to: &str,
    kind: EdgeKind,
) -> &'a knect_proto::Edge {
    topology
        .edges
        .iter()
        .find(|edge| edge.kind == kind && edge.from.name == from && edge.to.name == to)
        .unwrap_or_else(|| panic!("missing edge {from} -> {to} {kind:?}"))
}

pub(crate) fn node<'a>(
    topology: &'a Topology,
    category: NodeCategory,
    name: &str,
) -> &'a knect_proto::ResourceNode {
    topology
        .nodes
        .iter()
        .find(|node| node.category == category && node.name == name)
        .unwrap_or_else(|| panic!("missing node {category:?}/{name}"))
}

pub(crate) fn badge_labels(node: &knect_proto::ResourceNode) -> Vec<&str> {
    node.badges
        .iter()
        .map(|badge| badge.label.as_str())
        .collect()
}

pub(crate) fn set_max_scale(store: &mut ResourceStore, service: &str, value: Option<&str>) {
    let id = ResourceId::namespaced("serving.knative.dev/v1", "Service", NAMESPACE, service);
    let mut object = (*store.get(&id).expect("service exists")).clone();
    let annotations = object
        .data
        .pointer_mut("/spec/template/metadata/annotations")
        .and_then(Value::as_object_mut)
        .expect("template annotations exist");
    match value {
        Some(value) => {
            annotations.insert("autoscaling.knative.dev/maxScale".to_owned(), json!(value));
        }
        None => {
            annotations.remove("autoscaling.knative.dev/maxScale");
        }
    }
    store.upsert(object);
}

pub(crate) fn set_claim_access_modes(store: &mut ResourceStore, claim: &str, modes: &[&str]) {
    let id = ResourceId::namespaced("v1", "PersistentVolumeClaim", NAMESPACE, claim);
    let mut object = (*store.get(&id).expect("claim exists")).clone();
    if let Some(spec) = object.data.get_mut("spec").and_then(Value::as_object_mut) {
        spec.insert("accessModes".to_owned(), json!(modes));
    }
    store.upsert(object);
}

pub(crate) fn conflict_warnings(topology: &Topology) -> Vec<&knect_proto::graph::Warning> {
    topology
        .warnings
        .iter()
        .filter(|warning| warning.code == "rwo_maxscale_conflict")
        .collect()
}
