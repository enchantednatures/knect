//! Resolution of resource references and owner chains to a Knative Service.

use std::collections::BTreeSet;

use knect_proto::ResourceId;
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::graph::support::reference_id;
use crate::store::ResourceStore;

use super::support::{FLAGGER_GROUP, SERVING_API, SERVING_GROUP, ids_by_group_kind};

/// Resolves a serialized resource reference, falling back to its UID.
#[must_use]
pub fn scope_id(
    store: &ResourceStore,
    reference: &Value,
    default_namespace: Option<&str>,
) -> Option<ResourceId> {
    if let Some(id) = reference_id(reference, default_namespace) {
        return Some(id);
    }
    reference
        .get("uid")
        .and_then(Value::as_str)
        .and_then(|uid| find_by_uid(store, uid))
}

/// Finds a stored object by Kubernetes UID.
#[must_use]
pub fn find_by_uid(store: &ResourceStore, uid: &str) -> Option<ResourceId> {
    store.id_by_uid(uid)
}

/// Walks UIDs, owner references, labels, and name prefixes to the owning
/// Knative Service.
#[must_use]
pub fn parent_kservice(store: &ResourceStore, id: &ResourceId) -> Option<ResourceId> {
    let mut current = id.clone();
    let mut visited = BTreeSet::new();
    for _ in 0..16 {
        if !visited.insert(current.clone()) {
            break;
        }
        if is_kservice(&current) {
            return store.contains(&current).then_some(current);
        }
        let Some(object) = store.get(&current) else {
            return name_based_parent(store, &current);
        };
        if let Some(found) = from_labels(store, &current, &object) {
            return Some(found);
        }
        if let Some(found) = from_revision_prefix(store, &current) {
            return Some(found);
        }
        if let Some(found) = from_canary(store, &current, &object) {
            return Some(found);
        }
        let Some(owner) = first_owner(&current, &object) else {
            break;
        };
        current = owner;
    }
    None
}

fn is_kservice(id: &ResourceId) -> bool {
    id.kind == "Service" && id.gvk().group == SERVING_GROUP
}

fn kservice(store: &ResourceStore, namespace: Option<&str>, name: &str) -> Option<ResourceId> {
    let id = ResourceId::new(
        SERVING_API,
        "Service",
        namespace.map(ToOwned::to_owned),
        name,
    );
    store.contains(&id).then_some(id)
}

fn from_labels(
    store: &ResourceStore,
    current: &ResourceId,
    object: &DynamicObject,
) -> Option<ResourceId> {
    let labels = object.labels();
    if let Some(service) = labels.get("serving.knative.dev/service")
        && let Some(found) = kservice(store, current.namespace.as_deref(), service)
    {
        return Some(found);
    }
    if let Some(service) = labels.get("serving.knative.dev/configuration")
        && let Some(found) = kservice(store, current.namespace.as_deref(), service)
    {
        return Some(found);
    }
    labels
        .get("serving.knative.dev/revision")
        .and_then(|revision| revision_parent(store, current.namespace.as_deref(), revision))
}

fn revision_parent(
    store: &ResourceStore,
    namespace: Option<&str>,
    revision: &str,
) -> Option<ResourceId> {
    let id = ResourceId::new(
        SERVING_API,
        "Revision",
        namespace.map(ToOwned::to_owned),
        revision,
    );
    let object = store.get(&id)?;
    let service = object.labels().get("serving.knative.dev/service")?;
    kservice(store, namespace, service)
}

fn from_revision_prefix(store: &ResourceStore, current: &ResourceId) -> Option<ResourceId> {
    let revisions = ids_by_group_kind(store, SERVING_GROUP, "Revision");
    let longest = revisions
        .into_iter()
        .filter(|revision| revision.namespace == current.namespace)
        .filter(|revision| {
            current.name == revision.name
                || current
                    .name
                    .strip_prefix(&revision.name)
                    .is_some_and(|rest| rest.starts_with('-'))
        })
        .max_by_key(|revision| revision.name.len())?;
    revision_parent(store, longest.namespace.as_deref(), &longest.name)
}

fn from_canary(
    store: &ResourceStore,
    current: &ResourceId,
    _object: &DynamicObject,
) -> Option<ResourceId> {
    let canaries = ids_by_group_kind(store, FLAGGER_GROUP, "Canary");
    for canary_id in canaries {
        if canary_id.namespace != current.namespace {
            continue;
        }
        let Some(canary) = store.get(&canary_id) else {
            continue;
        };
        if target_matches(&canary, current) {
            let target = canary
                .data
                .get("spec")
                .and_then(|spec| spec.get("targetRef"))
                .and_then(|reference| reference_id(reference, current.namespace.as_deref()));
            if target.as_ref().is_some_and(is_kservice) {
                return target;
            }
            return kservice_by_prefix(store, current.namespace.as_deref(), &canary_id.name);
        }
    }
    None
}

fn target_matches(canary: &DynamicObject, current: &ResourceId) -> bool {
    let Some(reference) = canary
        .data
        .get("spec")
        .and_then(|spec| spec.get("targetRef"))
    else {
        return false;
    };
    let target = reference_id(reference, current.namespace.as_deref());
    target.as_ref() == Some(current)
}

fn name_based_parent(store: &ResourceStore, current: &ResourceId) -> Option<ResourceId> {
    from_revision_prefix(store, current)
        .or_else(|| kservice_by_prefix(store, current.namespace.as_deref(), &current.name))
}

fn kservice_by_prefix(
    store: &ResourceStore,
    namespace: Option<&str>,
    name: &str,
) -> Option<ResourceId> {
    let services = ids_by_group_kind(store, SERVING_GROUP, "Service");
    services
        .into_iter()
        .filter(|service| service.namespace.as_deref() == namespace)
        .filter(|service| {
            name == service.name
                || name
                    .strip_prefix(&service.name)
                    .is_some_and(|rest| rest.starts_with('-'))
        })
        .max_by_key(|service| service.name.len())
}

fn first_owner(current: &ResourceId, object: &DynamicObject) -> Option<ResourceId> {
    let owner = object.owner_references().first()?;
    if owner.name.is_empty() {
        return None;
    }
    let id = ResourceId::new(
        owner.api_version.clone(),
        owner.kind.clone(),
        current.namespace.clone(),
        owner.name.clone(),
    );
    (id != *current).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource::tests::test_object;

    #[test]
    fn revision_name_prefix_maps_deployments_to_the_service() {
        let mut store = ResourceStore::new();
        let mut revision = test_object(SERVING_API, "Revision", Some("default"), "order-00002");
        revision.metadata.labels = Some(
            [("serving.knative.dev/service".to_owned(), "order".to_owned())]
                .into_iter()
                .collect(),
        );
        store.upsert(revision);
        let mut service = test_object(SERVING_API, "Service", Some("default"), "order");
        service.metadata.uid = Some("uid-order".to_owned());
        store.upsert(service);

        let deployment =
            ResourceId::namespaced("apps/v1", "Deployment", "default", "order-00002-deployment");
        assert_eq!(
            parent_kservice(&store, &deployment),
            Some(ResourceId::namespaced(
                SERVING_API,
                "Service",
                "default",
                "order"
            ))
        );
    }

    #[test]
    fn uid_lookup_finds_a_stored_object() {
        let mut store = ResourceStore::new();
        let mut service = test_object(SERVING_API, "Service", Some("default"), "order");
        service.metadata.uid = Some("uid-order".to_owned());
        store.upsert(service);
        assert_eq!(
            find_by_uid(&store, "uid-order"),
            Some(ResourceId::namespaced(
                SERVING_API,
                "Service",
                "default",
                "order"
            ))
        );
        assert_eq!(find_by_uid(&store, "missing"), None);
    }
}
