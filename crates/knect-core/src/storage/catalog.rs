//! Indexing and resolution of persistent storage objects.
//!
//! Every function here is pure over the sanitized [`ResourceStore`] snapshot:
//! no Kubernetes client, no I/O, and deterministic iteration order.

use std::collections::BTreeMap;

use knect_proto::storage::AccessMode;
use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;
use serde_json::Value;

use crate::correlate::CorrelationContext;
use crate::graph::support::{field, text_field};
use crate::storage::parse_access_modes;
use crate::store::ResourceStore;

use super::backend::{default_storage_class, index_ceph, resolve_class_backends};
use super::mounts::{Mount, collect_mounts};

/// Resolved backing relationship for one claim.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaimRelation {
    /// Bound persistent volume.
    pub pv: Option<ResourceId>,
    /// StorageClass that provisioned the volume.
    pub storage_class: Option<ResourceId>,
    /// Rook Ceph pool or filesystem behind the StorageClass.
    pub ceph: Option<ResourceId>,
}

/// Sanitized claim facts used for node summaries and storage records.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaimFacts {
    /// Access modes declared by the claim.
    pub access_modes: Vec<AccessMode>,
    /// Requested or actual capacity.
    pub capacity: Option<String>,
    /// Bound phase (`Bound`, `Pending`, `Lost`).
    pub phase: Option<String>,
    /// StorageClass name declared by the claim or its volume.
    pub storage_class_name: Option<String>,
    /// Backing relationship.
    pub relation: ClaimRelation,
}

/// Storage objects and relationships discovered in the store.
#[derive(Debug, Clone, Default)]
pub(crate) struct Catalog {
    /// StorageClass identities, sorted.
    pub storage_classes: Vec<ResourceId>,
    /// PersistentVolume identities, sorted.
    pub persistent_volumes: Vec<ResourceId>,
    /// PersistentVolumeClaim identities, sorted.
    pub persistent_volume_claims: Vec<ResourceId>,
    /// CephBlockPool and CephFilesystem identities, sorted.
    pub ceph_pools: Vec<ResourceId>,
    /// Claim facts keyed by claim identity.
    pub claims: BTreeMap<ResourceId, ClaimFacts>,
    /// Claim backing relationships.
    pub claim_relations: BTreeMap<ResourceId, ClaimRelation>,
    /// PersistentVolume to StorageClass mapping.
    pub volume_storage_classes: BTreeMap<ResourceId, ResourceId>,
    /// StorageClass to Rook Ceph backend mapping.
    pub class_backends: BTreeMap<ResourceId, ResourceId>,
    /// Volume mounts observed on workloads.
    pub mounts: Vec<Mount>,
}

/// Builds the deterministic storage catalog for one correlation context.
pub(crate) fn catalog(ctx: &CorrelationContext) -> Catalog {
    let store = &ctx.store;
    let storage_classes = store.ids_by_gvk(&Gvk::new("storage.k8s.io", "v1", "StorageClass"));
    let persistent_volumes = store.ids_by_gvk(&Gvk::new("", "v1", "PersistentVolume"));
    let persistent_volume_claims = store.ids_by_gvk(&Gvk::new("", "v1", "PersistentVolumeClaim"));
    let mut ceph_pools = store.ids_by_gvk(&Gvk::new("ceph.rook.io", "v1", "CephBlockPool"));
    ceph_pools.extend(store.ids_by_gvk(&Gvk::new("ceph.rook.io", "v1", "CephFilesystem")));
    ceph_pools.sort();

    let sc_by_name = index_by_name(store, &storage_classes);
    let pv_by_name = index_by_name(store, &persistent_volumes);
    let ceph_index = index_ceph(store, &ceph_pools);
    let default_class = default_storage_class(store, &storage_classes);
    let default_class_name = default_class.as_ref().map(|id| id.name.clone());

    let class_backends = resolve_class_backends(store, &storage_classes, &ceph_index);
    let volume_storage_classes = resolve_volume_classes(store, &persistent_volumes, &sc_by_name);
    let claims = resolve_claims(
        store,
        &persistent_volume_claims,
        &persistent_volumes,
        &pv_by_name,
        &sc_by_name,
        default_class_name.as_deref(),
        &class_backends,
    );
    let claim_relations = claims
        .iter()
        .map(|(id, facts)| (id.clone(), facts.relation.clone()))
        .collect();
    let mounts = collect_mounts(store);

    Catalog {
        storage_classes,
        persistent_volumes,
        persistent_volume_claims,
        ceph_pools,
        claims,
        claim_relations,
        volume_storage_classes,
        class_backends,
        mounts,
    }
}

fn index_by_name(store: &ResourceStore, ids: &[ResourceId]) -> BTreeMap<String, ResourceId> {
    ids.iter()
        .filter(|id| !id.name.is_empty() && store.contains(id))
        .map(|id| (id.name.clone(), id.clone()))
        .collect()
}

fn resolve_volume_classes(
    store: &ResourceStore,
    persistent_volumes: &[ResourceId],
    sc_by_name: &BTreeMap<String, ResourceId>,
) -> BTreeMap<ResourceId, ResourceId> {
    let mut mapping = BTreeMap::new();
    for volume_id in persistent_volumes {
        let Some(object) = store.get(volume_id) else {
            continue;
        };
        let Some(name) =
            text_field(&object, &["spec", "storageClassName"]).filter(|name| !name.is_empty())
        else {
            continue;
        };
        if let Some(class_id) = sc_by_name.get(name) {
            mapping.insert(volume_id.clone(), class_id.clone());
        }
    }
    mapping
}

fn resolve_claims(
    store: &ResourceStore,
    claims: &[ResourceId],
    persistent_volumes: &[ResourceId],
    pv_by_name: &BTreeMap<String, ResourceId>,
    sc_by_name: &BTreeMap<String, ResourceId>,
    default_class: Option<&str>,
    class_backends: &BTreeMap<ResourceId, ResourceId>,
) -> BTreeMap<ResourceId, ClaimFacts> {
    let mut resolved = BTreeMap::new();
    for claim_id in claims {
        let Some(object) = store.get(claim_id) else {
            continue;
        };
        let volume_id = volume_for_claim(store, persistent_volumes, pv_by_name, claim_id, &object);
        let volume = volume_id.as_ref().and_then(|id| store.get(id));
        let class_name = claim_storage_class(&object, volume.as_deref(), default_class);
        let storage_class = class_name
            .as_deref()
            .and_then(|name| sc_by_name.get(name))
            .cloned();
        let ceph = storage_class
            .as_ref()
            .and_then(|id| class_backends.get(id))
            .cloned();
        let relation = ClaimRelation {
            pv: volume_id,
            storage_class,
            ceph,
        };
        resolved.insert(
            claim_id.clone(),
            ClaimFacts {
                access_modes: access_modes(&object),
                capacity: capacity(&object),
                phase: text_field(&object, &["status", "phase"]).map(ToOwned::to_owned),
                storage_class_name: class_name,
                relation,
            },
        );
    }
    resolved
}

fn volume_for_claim(
    store: &ResourceStore,
    persistent_volumes: &[ResourceId],
    pv_by_name: &BTreeMap<String, ResourceId>,
    claim_id: &ResourceId,
    object: &DynamicObject,
) -> Option<ResourceId> {
    if let Some(name) = text_field(object, &["spec", "volumeName"]).filter(|name| !name.is_empty())
        && let Some(id) = pv_by_name.get(name)
    {
        return Some(id.clone());
    }
    persistent_volumes
        .iter()
        .find(|volume_id| {
            let Some(volume) = store.get(volume_id) else {
                return false;
            };
            let claim_name = text_field(&volume, &["spec", "claimRef", "name"]);
            let claim_namespace = text_field(&volume, &["spec", "claimRef", "namespace"]);
            claim_name == Some(claim_id.name.as_str())
                && claim_namespace
                    .is_none_or(|namespace| Some(namespace) == claim_id.namespace.as_deref())
        })
        .cloned()
}

fn claim_storage_class(
    object: &DynamicObject,
    volume: Option<&DynamicObject>,
    default_class: Option<&str>,
) -> Option<String> {
    let own = text_field(object, &["spec", "storageClassName"]).filter(|name| !name.is_empty());
    let from_volume = volume
        .and_then(|volume| text_field(volume, &["spec", "storageClassName"]))
        .filter(|name| !name.is_empty());
    own.or(from_volume)
        .map(ToOwned::to_owned)
        .or_else(|| default_class.map(ToOwned::to_owned))
}

fn access_modes(object: &DynamicObject) -> Vec<AccessMode> {
    let declared = field(object, &["spec", "accessModes"]).and_then(Value::as_array);
    let observed = field(object, &["status", "accessModes"]).and_then(Value::as_array);
    let values = declared
        .filter(|values| !values.is_empty())
        .or(observed)
        .map(Vec::as_slice)
        .unwrap_or_default();
    parse_access_modes(values.iter().filter_map(Value::as_str))
}

fn capacity(object: &DynamicObject) -> Option<String> {
    text_field(object, &["status", "capacity", "storage"])
        .or_else(|| text_field(object, &["spec", "resources", "requests", "storage"]))
        .filter(|capacity| !capacity.is_empty())
        .map(ToOwned::to_owned)
}
