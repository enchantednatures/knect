#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Fixture-driven persistent storage correlation tests.

mod storage_support;

use std::sync::Arc;

use knect_core::correlate::CorrelationContext;
use knect_core::storage::{AccessMode, StorageResolver, StoreStorageResolver};
use knect_proto::graph::{EdgeKind, Health};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::{Value, json};

use storage_support::*;

#[test]
fn fixture_produces_exact_storage_nodes() {
    let topology = build_topology(&load_store());
    let mut nodes: Vec<(NodeCategory, String)> = topology
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.category,
                NodeCategory::StorageClass
                    | NodeCategory::PersistentVolume
                    | NodeCategory::PersistentVolumeClaim
                    | NodeCategory::CephPool
            )
        })
        .map(|node| (node.category, node.name.clone()))
        .collect();
    nodes.sort();
    let mut expected = vec![
        (NodeCategory::StorageClass, "rook-ceph-block".to_owned()),
        (NodeCategory::StorageClass, "rook-cephfs".to_owned()),
        (
            NodeCategory::PersistentVolume,
            "pvc-storage-data".to_owned(),
        ),
        (
            NodeCategory::PersistentVolume,
            "pvc-shared-assets".to_owned(),
        ),
        (NodeCategory::PersistentVolume, "pvc-single-data".to_owned()),
        (
            NodeCategory::PersistentVolumeClaim,
            "storage-data".to_owned(),
        ),
        (
            NodeCategory::PersistentVolumeClaim,
            "shared-assets".to_owned(),
        ),
        (
            NodeCategory::PersistentVolumeClaim,
            "single-data".to_owned(),
        ),
        (NodeCategory::CephPool, "replicated".to_owned()),
        (NodeCategory::CephPool, "cephfs".to_owned()),
    ];
    expected.sort();
    assert_eq!(nodes, expected);
}

#[test]
fn fixture_produces_exact_storage_edges() {
    let topology = build_topology(&load_store());
    let mut expected = vec![
        (
            serving_id("Revision", "storage-consumer-00001"),
            core_id("PersistentVolumeClaim", "storage-data"),
            EdgeKind::Mounts,
        ),
        (
            apps_id("Deployment", "storage-consumer-00001-deployment"),
            core_id("PersistentVolumeClaim", "storage-data"),
            EdgeKind::Mounts,
        ),
        (
            core_id("Pod", "storage-consumer-00001-abc"),
            core_id("PersistentVolumeClaim", "storage-data"),
            EdgeKind::Mounts,
        ),
        (
            serving_id("Revision", "storage-reader-00001"),
            core_id("PersistentVolumeClaim", "shared-assets"),
            EdgeKind::Mounts,
        ),
        (
            serving_id("Revision", "storage-single-00001"),
            core_id("PersistentVolumeClaim", "single-data"),
            EdgeKind::Mounts,
        ),
        (
            core_id("PersistentVolumeClaim", "storage-data"),
            cluster_id("v1", "PersistentVolume", "pvc-storage-data"),
            EdgeKind::BackedBy,
        ),
        (
            core_id("PersistentVolumeClaim", "shared-assets"),
            cluster_id("v1", "PersistentVolume", "pvc-shared-assets"),
            EdgeKind::BackedBy,
        ),
        (
            core_id("PersistentVolumeClaim", "single-data"),
            cluster_id("v1", "PersistentVolume", "pvc-single-data"),
            EdgeKind::BackedBy,
        ),
        (
            cluster_id("v1", "PersistentVolume", "pvc-storage-data"),
            cluster_id("storage.k8s.io/v1", "StorageClass", "rook-ceph-block"),
            EdgeKind::ProvisionedBy,
        ),
        (
            cluster_id("v1", "PersistentVolume", "pvc-single-data"),
            cluster_id("storage.k8s.io/v1", "StorageClass", "rook-ceph-block"),
            EdgeKind::ProvisionedBy,
        ),
        (
            cluster_id("v1", "PersistentVolume", "pvc-shared-assets"),
            cluster_id("storage.k8s.io/v1", "StorageClass", "rook-cephfs"),
            EdgeKind::ProvisionedBy,
        ),
        (
            cluster_id("storage.k8s.io/v1", "StorageClass", "rook-ceph-block"),
            ceph_id("CephBlockPool", "replicated"),
            EdgeKind::BackedBy,
        ),
        (
            cluster_id("storage.k8s.io/v1", "StorageClass", "rook-cephfs"),
            ceph_id("CephFilesystem", "cephfs"),
            EdgeKind::BackedBy,
        ),
    ];
    expected.sort();
    assert_eq!(storage_edges(&topology), expected);
}

#[test]
fn storage_mount_edges_carry_volume_mount_path_and_access_modes() {
    let topology = build_topology(&load_store());
    let mount = edge(
        &topology,
        "storage-consumer-00001",
        "storage-data",
        EdgeKind::Mounts,
    );
    assert_eq!(mount.label.as_deref(), Some("static-data"));
    assert_eq!(
        mount.metadata.get("volume").and_then(Value::as_str),
        Some("static-data")
    );
    assert_eq!(
        mount.metadata.get("mount_path").and_then(Value::as_str),
        Some("/usr/share/nginx/html/data")
    );
    assert_eq!(
        mount.metadata.get("access_modes"),
        Some(&json!(["ReadWriteOnce"]))
    );
    assert_eq!(mount.health, Health::Degraded);
}

#[test]
fn storage_removing_the_volume_falls_back_to_the_storage_class_edge() {
    let mut store = load_store();
    let volume = ResourceId::cluster_scoped("v1", "PersistentVolume", "pvc-storage-data");
    store.remove(&volume);
    let topology = build_topology(&store);

    assert!(!topology.nodes.iter().any(|node| node.id == volume));
    assert!(
        topology
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::ProvisionedBy
                && edge.from.name == "storage-data"
                && edge.to.name == "rook-ceph-block")
    );
}

#[test]
fn storage_resolver_records_trace_claim_relationships_and_conflict() {
    let store = load_store();
    let ctx = CorrelationContext::new("fixture", 1, Arc::new(store));
    let records = StoreStorageResolver.records(&ctx).expect("records resolve");

    let data = records
        .iter()
        .find(|record| record.pvc.name == "storage-data")
        .expect("storage-data record");
    assert_eq!(data.storage_class.as_deref(), Some("rook-ceph-block"));
    assert_eq!(data.access_modes, vec![AccessMode::ReadWriteOnce]);
    assert_eq!(data.capacity.as_deref(), Some("5Gi"));
    assert_eq!(data.phase.as_deref(), Some("Bound"));
    assert_eq!(data.mounted_by.len(), 3);
    assert_eq!(
        data.provisioned_by.as_ref().map(|id| id.name.as_str()),
        Some("rook-ceph-block")
    );
    assert_eq!(
        data.backend.as_ref().map(|id| id.name.as_str()),
        Some("replicated")
    );
    assert_eq!(data.warnings.len(), 1);
    let conflict = data.warnings[0].conflict.as_ref().expect("conflict detail");
    assert_eq!(conflict.access_mode, AccessMode::ReadWriteOnce);
    assert_eq!(conflict.max_scale, Some(5));
    assert_eq!(conflict.affected.len(), 3);
    assert_eq!(conflict.pvc.name, "storage-data");
    assert!(conflict.reason.contains("maxScale=5"));

    let shared = records
        .iter()
        .find(|record| record.pvc.name == "shared-assets")
        .expect("shared-assets record");
    assert_eq!(shared.access_modes, vec![AccessMode::ReadWriteMany]);
    assert!(shared.warnings.is_empty());
    assert_eq!(
        shared.backend.as_ref().map(|id| id.name.as_str()),
        Some("cephfs")
    );
}

#[test]
fn storage_repeated_runs_are_deterministic() {
    let store = load_store();
    assert_eq!(build_topology(&store), build_topology(&store));
}

#[test]
fn storage_nodes_have_no_placeholders() {
    let topology = build_topology(&load_store());
    for node in &topology.nodes {
        assert!(!node.id.api_version.is_empty(), "{:?}", node.id);
        assert!(!node.id.kind.is_empty(), "{:?}", node.id);
        assert!(!node.id.name.is_empty(), "{:?}", node.id);
        assert_ne!(node.category, NodeCategory::Unknown, "{:?}", node.id);
    }
}
