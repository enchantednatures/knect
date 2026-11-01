//! Storage trace assembly.
//!
//! Combines the resolved storage catalog with the autoscaling linter and
//! produces the records returned by [`crate::storage::StorageResolver`].

use std::collections::{BTreeMap, BTreeSet};

use knect_proto::ResourceId;
use knect_proto::graph::Severity;
use knect_proto::storage::{AccessMode, StorageConflictWarning, StorageRecord, StorageWarning};

use crate::correlate::CorrelationContext;
use crate::storage::{access_mode_label, conflicts};

use super::catalog::{Catalog, ClaimFacts, ClaimRelation, catalog};
use super::lint::{CONFLICT_CODE, Conflict, conflict_reason};
use super::mounts::Mount;

/// One storage trace for a graph generation.
#[derive(Debug, Clone, Default)]
pub(crate) struct StorageTrace {
    /// StorageClass nodes to project.
    pub storage_classes: Vec<ResourceId>,
    /// PersistentVolume nodes to project.
    pub persistent_volumes: Vec<ResourceId>,
    /// PersistentVolumeClaim nodes to project.
    pub persistent_volume_claims: Vec<ResourceId>,
    /// CephBlockPool and CephFilesystem nodes to project.
    pub ceph_pools: Vec<ResourceId>,
    /// Storage records for node inspection.
    pub records: Vec<StorageRecord>,
    /// Volume mounts observed on workloads.
    pub mounts: Vec<Mount>,
    /// Autoscaling conflicts.
    pub conflicts: Vec<Conflict>,
    /// Claim facts keyed by claim identity.
    pub claims: BTreeMap<ResourceId, ClaimFacts>,
    /// Claim backing relationships.
    pub claim_relations: BTreeMap<ResourceId, ClaimRelation>,
    /// PersistentVolume to StorageClass mapping.
    pub volume_storage_classes: BTreeMap<ResourceId, ResourceId>,
    /// StorageClass to Ceph backend mapping.
    pub class_backends: BTreeMap<ResourceId, ResourceId>,
}

/// Traces persistent storage for one correlation context.
pub(crate) fn trace(ctx: &CorrelationContext) -> StorageTrace {
    let catalog = catalog(ctx);
    let access_modes = access_modes(&catalog);
    let conflicts = conflicts(&ctx.store, &access_modes, &catalog.mounts);
    let records = build_records(&catalog, &conflicts);

    StorageTrace {
        storage_classes: catalog.storage_classes,
        persistent_volumes: catalog.persistent_volumes,
        persistent_volume_claims: catalog.persistent_volume_claims,
        ceph_pools: catalog.ceph_pools,
        records,
        mounts: catalog.mounts,
        conflicts,
        claims: catalog.claims,
        claim_relations: catalog.claim_relations,
        volume_storage_classes: catalog.volume_storage_classes,
        class_backends: catalog.class_backends,
    }
}

fn access_modes(catalog: &Catalog) -> BTreeMap<ResourceId, Vec<AccessMode>> {
    catalog
        .claims
        .iter()
        .map(|(id, facts)| (id.clone(), facts.access_modes.clone()))
        .collect()
}

fn build_records(catalog: &Catalog, conflicts: &[Conflict]) -> Vec<StorageRecord> {
    catalog
        .claims
        .iter()
        .map(|(pvc, facts)| {
            let mounted_by = mounted_by(catalog, pvc);
            let volume = catalog
                .mounts
                .iter()
                .find(|mount| &mount.pvc == pvc)
                .map(|mount| mount.volume.clone());
            StorageRecord {
                pvc: pvc.clone(),
                volume,
                storage_class: facts.storage_class_name.clone(),
                access_modes: facts.access_modes.clone(),
                capacity: facts.capacity.clone(),
                phase: facts.phase.clone(),
                mounted_by,
                provisioned_by: facts.relation.storage_class.clone(),
                backend: facts.relation.ceph.clone(),
                warnings: warnings_for(pvc, conflicts),
            }
        })
        .collect()
}

fn mounted_by(catalog: &Catalog, pvc: &ResourceId) -> Vec<ResourceId> {
    let mut mounted: Vec<ResourceId> = catalog
        .mounts
        .iter()
        .filter(|mount| &mount.pvc == pvc)
        .map(|mount| mount.workload.clone())
        .collect();
    mounted.sort();
    mounted.dedup();
    mounted
}

fn warnings_for(pvc: &ResourceId, conflicts: &[Conflict]) -> Vec<StorageWarning> {
    let mut groups: BTreeMap<(AccessMode, Option<u32>), BTreeSet<ResourceId>> = BTreeMap::new();
    for conflict in conflicts.iter().filter(|conflict| &conflict.pvc == pvc) {
        groups
            .entry((conflict.access_mode, conflict.max_scale))
            .or_default()
            .insert(conflict.workload.clone());
    }

    groups
        .into_iter()
        .map(|((access_mode, max_scale), affected)| {
            let reason = conflict_reason(access_mode, max_scale);
            let message = format!(
                "{} claim {} is mounted by consumers that may exceed one replica",
                access_mode_label(access_mode),
                pvc.name
            );
            StorageWarning {
                code: CONFLICT_CODE.to_owned(),
                message,
                severity: Severity::High,
                conflict: Some(StorageConflictWarning {
                    pvc: pvc.clone(),
                    access_mode,
                    max_scale,
                    reason,
                    affected: affected.into_iter().collect(),
                }),
            }
        })
        .collect()
}
