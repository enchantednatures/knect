//! `storage` scenario.
//!
//! Traces the Revision -> PVC -> PV -> StorageClass -> Ceph chain and verifies
//! the ReadWriteOnce/maxScale linter result exposed through node details.

use knect_proto::graph::{EdgeKind, NodeDetails, Severity};
use knect_proto::storage::{AccessMode, StorageRecord};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{
    ScenarioError, badge_labels, fixture, protocol, require_edge, require_node,
};

const CONFLICT_CODE: &str = "rwo_maxscale_conflict";
const CONFLICT_BADGE: &str = "rwo-maxscale-conflict";

/// Runs the storage scenario against the isolated storage fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("storage").await?;
    let topology = harness.topology().await?;

    for (category, name) in [
        (NodeCategory::PersistentVolumeClaim, "storage-data"),
        (NodeCategory::PersistentVolumeClaim, "shared-assets"),
        (NodeCategory::PersistentVolumeClaim, "single-data"),
        (NodeCategory::PersistentVolume, "pvc-storage-data"),
        (NodeCategory::StorageClass, "rook-ceph-block"),
        (NodeCategory::StorageClass, "rook-cephfs"),
        (NodeCategory::CephPool, "replicated"),
        (NodeCategory::CephPool, "cephfs"),
    ] {
        require_node(&topology, category, name)?;
    }

    require_edge(
        &topology,
        "storage-consumer-00001",
        "storage-data",
        EdgeKind::Mounts,
    )?;
    require_edge(
        &topology,
        "storage-data",
        "pvc-storage-data",
        EdgeKind::BackedBy,
    )?;
    require_edge(
        &topology,
        "pvc-storage-data",
        "rook-ceph-block",
        EdgeKind::ProvisionedBy,
    )?;
    require_edge(
        &topology,
        "rook-ceph-block",
        "replicated",
        EdgeKind::BackedBy,
    )?;

    let claim = require_node(
        &topology,
        NodeCategory::PersistentVolumeClaim,
        "storage-data",
    )?;
    if !claim.summary.contains("5Gi") || !claim.summary.contains("rook-ceph-block") {
        return Err(protocol(format!(
            "PVC summary lost capacity or storage class: {}",
            claim.summary
        )));
    }

    let service = require_node(&topology, NodeCategory::KService, "storage-consumer")?;
    let revision = require_node(&topology, NodeCategory::Revision, "storage-consumer-00001")?;
    for node in [service, revision] {
        if !badge_labels(node).contains(&CONFLICT_BADGE) {
            return Err(protocol(format!(
                "{} is missing the {CONFLICT_BADGE} badge",
                node.name
            )));
        }
    }

    let warning = topology
        .warnings
        .iter()
        .find(|warning| warning.code == CONFLICT_CODE)
        .ok_or_else(|| protocol("topology has no RWO/maxScale warning"))?;
    if warning.severity != Severity::High {
        return Err(protocol(format!(
            "RWO warning severity is {:?}, expected high",
            warning.severity
        )));
    }
    if !warning.message.contains("maxScale=5") {
        return Err(protocol(format!(
            "RWO warning does not name maxScale=5: {}",
            warning.message
        )));
    }

    let revision_id = ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Revision",
        "knect-e2e",
        "storage-consumer-00001",
    );
    let details: NodeDetails = serde_json::from_value(
        harness
            .call(
                "graph.get_node_details",
                Some(json!({"id": revision_id, "include_yaml": true})),
            )
            .await?,
    )
    .map_err(|error| protocol(format!("invalid node details: {error}")))?;

    let record = details
        .storage
        .iter()
        .find(|record| record.pvc.name == "storage-data")
        .ok_or_else(|| protocol("node details do not include the storage-data claim"))?;
    assert_conflict(record)?;
    if details
        .warnings
        .iter()
        .all(|warning| warning.code != CONFLICT_CODE)
    {
        return Err(protocol(
            "node details warnings do not include the RWO/maxScale conflict",
        ));
    }

    daemon.shutdown().await?;
    Ok(())
}

fn assert_conflict(record: &StorageRecord) -> Result<(), ScenarioError> {
    if record.access_modes != vec![AccessMode::ReadWriteOnce] {
        return Err(protocol(format!(
            "storage-data access modes are {:?}",
            record.access_modes
        )));
    }
    if record.mounted_by.is_empty() {
        return Err(protocol("storage-data record has no mounted_by entries"));
    }
    let warning = record
        .warnings
        .iter()
        .find(|warning| warning.code == CONFLICT_CODE)
        .ok_or_else(|| protocol("storage-data record has no conflict warning"))?;
    let conflict = warning
        .conflict
        .as_ref()
        .ok_or_else(|| protocol("storage-data warning has no conflict detail"))?;
    if conflict.max_scale != Some(5) {
        return Err(protocol(format!(
            "storage-data conflict max_scale is {:?}, expected 5",
            conflict.max_scale
        )));
    }
    if conflict.affected.is_empty() {
        return Err(protocol(
            "storage-data conflict does not list affected consumers",
        ));
    }
    if !warning.message.contains("maxScale=5") {
        return Err(protocol(format!(
            "storage-data conflict warning lacks maxScale=5: {}",
            warning.message
        )));
    }
    Ok(())
}
