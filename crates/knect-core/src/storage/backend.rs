//! Rook Ceph backend resolution from StorageClass metadata.

use std::collections::BTreeMap;

use knect_proto::ResourceId;
use kube::api::DynamicObject;
use serde_json::{Map, Value};

use crate::graph::support::{field, text_field};
use crate::store::ResourceStore;

const DEFAULT_NAMESPACE_KEY: &str = "storageclass.kubernetes.io/is-default-class";

pub(crate) fn index_ceph(
    store: &ResourceStore,
    ids: &[ResourceId],
) -> BTreeMap<(String, String), Vec<ResourceId>> {
    let mut index: BTreeMap<(String, String), Vec<ResourceId>> = BTreeMap::new();
    for id in ids {
        if id.name.is_empty() || !store.contains(id) {
            continue;
        }
        index
            .entry((id.kind.clone(), id.name.clone()))
            .or_default()
            .push(id.clone());
    }
    index
}

pub(crate) fn default_storage_class(
    store: &ResourceStore,
    ids: &[ResourceId],
) -> Option<ResourceId> {
    ids.iter()
        .find(|id| {
            store.get(id).is_some_and(|object| {
                annotation(&object, DEFAULT_NAMESPACE_KEY) == Some("true")
                    || annotation(&object, "storageclass.beta.kubernetes.io/is-default-class")
                        == Some("true")
            })
        })
        .cloned()
}

pub(crate) fn resolve_class_backends(
    store: &ResourceStore,
    storage_classes: &[ResourceId],
    ceph_index: &BTreeMap<(String, String), Vec<ResourceId>>,
) -> BTreeMap<ResourceId, ResourceId> {
    let mut backends = BTreeMap::new();
    for class_id in storage_classes {
        let Some(object) = store.get(class_id) else {
            continue;
        };
        if let Some(ceph) = class_backend(&object, ceph_index) {
            backends.insert(class_id.clone(), ceph);
        }
    }
    backends
}

fn class_backend(
    object: &DynamicObject,
    ceph_index: &BTreeMap<(String, String), Vec<ResourceId>>,
) -> Option<ResourceId> {
    let provisioner = text_field(object, &["provisioner"]).unwrap_or_default();
    let parameters = field(object, &["parameters"]).and_then(Value::as_object);
    let namespace = metadata_value(object, "knect.dev/ceph-namespace")
        .or_else(|| metadata_value(object, "rook.io/namespace"))
        .or_else(|| parameter(parameters, "clusterNamespace"));

    let pool = metadata_value(object, "knect.dev/ceph-pool")
        .or_else(|| metadata_value(object, "ceph.rook.io/pool"))
        .or_else(|| parameter(parameters, "pool"));
    let filesystem = metadata_value(object, "knect.dev/ceph-filesystem")
        .or_else(|| metadata_value(object, "ceph.rook.io/filesystem"))
        .or_else(|| parameter(parameters, "fsName"))
        .or_else(|| parameter(parameters, "filesystem"));

    let (kind, name) = if provisioner.contains("cephfs") || filesystem.is_some() {
        ("CephFilesystem", filesystem.or(pool)?)
    } else if provisioner.contains("rbd") || pool.is_some() {
        ("CephBlockPool", pool?)
    } else {
        return None;
    };
    pick_ceph(ceph_index, kind, name, namespace)
}

fn pick_ceph(
    ceph_index: &BTreeMap<(String, String), Vec<ResourceId>>,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Option<ResourceId> {
    let candidates = ceph_index.get(&(kind.to_owned(), name.to_owned()))?;
    if let Some(namespace) = namespace
        && let Some(id) = candidates
            .iter()
            .find(|id| id.namespace.as_deref() == Some(namespace))
    {
        return Some(id.clone());
    }
    if let Some(id) = candidates
        .iter()
        .find(|id| id.namespace.as_deref() == Some("rook-ceph"))
    {
        return Some(id.clone());
    }
    candidates.first().cloned()
}

fn annotation<'a>(object: &'a DynamicObject, key: &str) -> Option<&'a str> {
    object
        .metadata
        .annotations
        .as_ref()?
        .get(key)
        .map(String::as_str)
}

fn metadata_value<'a>(object: &'a DynamicObject, key: &str) -> Option<&'a str> {
    annotation(object, key).or_else(|| {
        object
            .metadata
            .labels
            .as_ref()?
            .get(key)
            .map(String::as_str)
    })
}

fn parameter<'a>(parameters: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a str> {
    parameters?
        .get(key)?
        .as_str()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceph_parameters_resolve_with_namespace_preference() {
        let mut index = BTreeMap::new();
        index.insert(
            ("CephBlockPool".to_owned(), "replicated".to_owned()),
            vec![
                ResourceId::namespaced(
                    "ceph.rook.io/v1",
                    "CephBlockPool",
                    "elsewhere",
                    "replicated",
                ),
                ResourceId::namespaced(
                    "ceph.rook.io/v1",
                    "CephBlockPool",
                    "rook-ceph",
                    "replicated",
                ),
            ],
        );
        assert_eq!(
            pick_ceph(&index, "CephBlockPool", "replicated", Some("elsewhere"))
                .and_then(|id| id.namespace),
            Some("elsewhere".to_owned())
        );
        assert_eq!(
            pick_ceph(&index, "CephBlockPool", "replicated", None).and_then(|id| id.namespace),
            Some("rook-ceph".to_owned())
        );
        assert!(pick_ceph(&index, "CephFilesystem", "missing", None).is_none());
    }
}
