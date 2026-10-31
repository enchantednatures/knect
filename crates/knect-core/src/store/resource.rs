//! Object identity and index keys derived from `DynamicObject` values.

use std::collections::BTreeMap;

use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;

/// Stable key for an owner reference, independent of the owning object's UID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerRefKey {
    /// Owner `apiVersion`.
    pub api_version: String,
    /// Owner kind.
    pub kind: String,
    /// Owner name.
    pub name: String,
}

impl OwnerRefKey {
    /// Creates an owner reference key.
    #[must_use]
    pub fn new(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
            name: name.into(),
        }
    }
}

/// Derives the canonical [`ResourceId`] for an object.
///
/// Missing type metadata or names produce empty components instead of a panic.
/// Task 3 rejects malformed objects before they reach the store; the fallback
/// keeps this boundary total.
#[must_use]
pub fn resource_id(object: &DynamicObject) -> ResourceId {
    let api_version = object
        .types
        .as_ref()
        .map(|types| types.api_version.clone())
        .or_else(|| text_field(object, "apiVersion"))
        .unwrap_or_default();
    let kind = object
        .types
        .as_ref()
        .map(|types| types.kind.clone())
        .or_else(|| text_field(object, "kind"))
        .unwrap_or_default();
    let name = object
        .metadata
        .name
        .clone()
        .or_else(|| nested_text_field(object, "metadata", "name"))
        .unwrap_or_default();
    let namespace = object
        .metadata
        .namespace
        .clone()
        .or_else(|| nested_text_field(object, "metadata", "namespace"));
    ResourceId::new(api_version, kind, namespace, name)
}

/// Returns the GVK of an object's canonical identity.
#[must_use]
pub fn object_gvk(object: &DynamicObject) -> Gvk {
    resource_id(object).gvk()
}

/// Returns owner UIDs referenced by the object, sorted and deduplicated.
#[must_use]
pub fn owner_uids(object: &DynamicObject) -> Vec<String> {
    let Some(references) = object.metadata.owner_references.as_deref() else {
        return Vec::new();
    };
    let mut uids: Vec<String> = references
        .iter()
        .map(|reference| reference.uid.clone())
        .filter(|uid| !uid.is_empty())
        .collect();
    uids.sort();
    uids.dedup();
    uids
}

/// Returns owner keys referenced by the object, sorted and deduplicated.
#[must_use]
pub fn owner_refs(object: &DynamicObject) -> Vec<OwnerRefKey> {
    let Some(references) = object.metadata.owner_references.as_deref() else {
        return Vec::new();
    };
    let mut keys: Vec<OwnerRefKey> = references
        .iter()
        .map(|reference| {
            OwnerRefKey::new(
                reference.api_version.clone(),
                reference.kind.clone(),
                reference.name.clone(),
            )
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Returns true when the object carries every selector label with an equal value.
#[must_use]
pub fn matches_labels(object: &DynamicObject, selector: &BTreeMap<String, String>) -> bool {
    let Some(labels) = object.metadata.labels.as_ref() else {
        return selector.is_empty();
    };
    selector
        .iter()
        .all(|(key, value)| labels.get(key).is_some_and(|actual| actual == value))
}

fn text_field(object: &DynamicObject, key: &str) -> Option<String> {
    object
        .data
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn nested_text_field(object: &DynamicObject, first: &str, second: &str) -> Option<String> {
    object
        .data
        .get(first)
        .and_then(|value| value.get(second))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use kube::api::{ObjectMeta, TypeMeta};

    pub(crate) fn test_object(
        api_version: &str,
        kind: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> DynamicObject {
        DynamicObject {
            types: Some(TypeMeta {
                api_version: api_version.to_owned(),
                kind: kind.to_owned(),
            }),
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: namespace.map(ToOwned::to_owned),
                ..ObjectMeta::default()
            },
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn identity_is_stable_for_the_same_object() {
        let first = test_object(
            "serving.knative.dev/v1",
            "Service",
            Some("default"),
            "hello",
        );
        let second = test_object(
            "serving.knative.dev/v1",
            "Service",
            Some("default"),
            "hello",
        );
        assert_eq!(resource_id(&first), resource_id(&second));
        assert_eq!(
            object_gvk(&first),
            Gvk::new("serving.knative.dev", "v1", "Service")
        );
    }

    #[test]
    fn namespaces_change_identity() {
        let default = test_object("v1", "Pod", Some("default"), "web");
        let other = test_object("v1", "Pod", Some("other"), "web");
        assert_ne!(resource_id(&default), resource_id(&other));
    }

    #[test]
    fn malformed_objects_have_total_identity() {
        let object = DynamicObject {
            types: None,
            metadata: Default::default(),
            data: serde_json::json!({}),
        };
        let id = resource_id(&object);
        assert_eq!(id.api_version, "");
        assert_eq!(id.kind, "");
        assert_eq!(id.name, "");
        assert!(id.namespace.is_none());
        assert!(owner_uids(&object).is_empty());
        assert!(owner_refs(&object).is_empty());
    }

    #[test]
    fn labels_match_is_subset_based() {
        let mut object = test_object("v1", "Pod", Some("default"), "web");
        object.metadata.labels = Some(
            [
                ("app".to_owned(), "web".to_owned()),
                ("tier".to_owned(), "frontend".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let mut selector = BTreeMap::new();
        selector.insert("app".to_owned(), "web".to_owned());
        assert!(matches_labels(&object, &selector));
        selector.insert("tier".to_owned(), "backend".to_owned());
        assert!(!matches_labels(&object, &selector));
        assert!(matches_labels(&object, &BTreeMap::new()));
    }
}
