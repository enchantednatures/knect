//! Ownership resolution from a workload up to its Knative Service.

use std::sync::Arc;

use knect_proto::{Gvk, ResourceId};
use kube::ResourceExt;
use kube::api::DynamicObject;

use crate::store::ResourceStore;

const SERVICE_LABEL: &str = "serving.knative.dev/service";
const MAX_OWNER_DEPTH: usize = 4;

pub(crate) fn owning_kservice(
    store: &ResourceStore,
    workload_id: &ResourceId,
) -> Option<ResourceId> {
    let services = store.ids_by_gvk(&Gvk::new("serving.knative.dev", "v1", "Service"));
    let object = store.get(workload_id)?;
    service_from_labels(&object, workload_id, &services)
        .or_else(|| walk_owners(store, &object, workload_id, &services, 0))
        .or_else(|| service_from_name(workload_id, &services))
}

fn service_from_labels(
    object: &DynamicObject,
    object_id: &ResourceId,
    services: &[ResourceId],
) -> Option<ResourceId> {
    let name = object.metadata.labels.as_ref()?.get(SERVICE_LABEL)?;
    find_service(services, object_id.namespace.as_deref(), name)
}

fn walk_owners(
    store: &ResourceStore,
    object: &DynamicObject,
    object_id: &ResourceId,
    services: &[ResourceId],
    depth: usize,
) -> Option<ResourceId> {
    if depth >= MAX_OWNER_DEPTH {
        return None;
    }
    for reference in object.owner_references() {
        if reference.kind == "Service"
            && reference.api_version.starts_with("serving.knative.dev")
            && let Some(service) =
                find_service(services, object_id.namespace.as_deref(), &reference.name)
        {
            return Some(service);
        }
        let owner_id = ResourceId::new(
            reference.api_version.clone(),
            reference.kind.clone(),
            object_id.namespace.clone(),
            reference.name.clone(),
        );
        let Some(owner) = store.get(&owner_id) else {
            continue;
        };
        if let Some(service) = service_from_labels(&owner, &owner_id, services)
            .or_else(|| walk_owners(store, &owner, &owner_id, services, depth + 1))
        {
            return Some(service);
        }
    }
    None
}

fn service_from_name(workload_id: &ResourceId, services: &[ResourceId]) -> Option<ResourceId> {
    let base = match workload_id.kind.as_str() {
        "Revision" => workload_id.name.as_str(),
        "Deployment" => workload_id.name.strip_suffix("-deployment")?,
        _ => return None,
    };
    services
        .iter()
        .filter(|service| service.namespace == workload_id.namespace)
        .filter(|service| base.starts_with(&format!("{}-", service.name)))
        .max_by_key(|service| service.name.len())
        .cloned()
}

fn find_service(
    services: &[ResourceId],
    namespace: Option<&str>,
    name: &str,
) -> Option<ResourceId> {
    services
        .iter()
        .find(|service| service.namespace.as_deref() == namespace && service.name == name)
        .cloned()
}

pub(crate) fn workload_revision(
    store: &ResourceStore,
    workload: &ResourceId,
) -> Option<Arc<DynamicObject>> {
    match workload.kind.as_str() {
        "Revision" => store.get(workload),
        "Deployment" => store.get(workload).and_then(|object| {
            object.owner_references().iter().find_map(|reference| {
                if reference.kind != "Revision" {
                    return None;
                }
                store.get(&ResourceId::new(
                    reference.api_version.clone(),
                    "Revision",
                    workload.namespace.clone(),
                    reference.name.clone(),
                ))
            })
        }),
        "Pod" => store
            .get(workload)
            .and_then(|object| pod_revision(store, &object, workload)),
        _ => None,
    }
}

fn pod_revision(
    store: &ResourceStore,
    object: &DynamicObject,
    workload: &ResourceId,
) -> Option<Arc<DynamicObject>> {
    if let Some(uid) = object
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get("serving.knative.dev/revisionUID"))
    {
        let revisions = store.ids_by_gvk(&Gvk::new("serving.knative.dev", "v1", "Revision"));
        for revision_id in revisions {
            if store
                .get(&revision_id)
                .is_some_and(|revision| revision.metadata.uid.as_ref() == Some(uid))
            {
                return store.get(&revision_id);
            }
        }
    }
    object.owner_references().iter().find_map(|reference| {
        let deployment_id = ResourceId::new(
            reference.api_version.clone(),
            reference.kind.clone(),
            workload.namespace.clone(),
            reference.name.clone(),
        );
        workload_revision(store, &deployment_id)
    })
}
