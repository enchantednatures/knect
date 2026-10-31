//! In-memory resource store with deterministic indexes and generations.
//!
//! The store owns sanitized `DynamicObject` values (Task 3 adds sanitization
//! before insertion). It is intentionally pure: no Kubernetes client, no I/O,
//! and no global state. Every mutation increments the generation.

pub mod resource;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;

pub use resource::{OwnerRefKey, matches_labels, object_gvk, owner_refs, owner_uids, resource_id};

/// A mutation observed by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreEvent {
    /// A new object entered the store.
    Added {
        /// Object identity.
        id: ResourceId,
        /// Store generation after the insertion.
        generation: u64,
    },
    /// An existing object was replaced.
    Updated {
        /// Object identity.
        id: ResourceId,
        /// Store generation after the update.
        generation: u64,
    },
    /// An object was removed.
    Removed {
        /// Object identity.
        id: ResourceId,
        /// Store generation after the removal.
        generation: u64,
    },
}

impl StoreEvent {
    /// Returns the object identity.
    #[must_use]
    pub fn id(&self) -> &ResourceId {
        match self {
            Self::Added { id, .. } | Self::Updated { id, .. } | Self::Removed { id, .. } => id,
        }
    }

    /// Returns the store generation after the change.
    #[must_use]
    pub fn generation(&self) -> u64 {
        match self {
            Self::Added { generation, .. }
            | Self::Updated { generation, .. }
            | Self::Removed { generation, .. } => *generation,
        }
    }

    /// Returns the frozen event kind name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Added { .. } => "added",
            Self::Updated { .. } => "updated",
            Self::Removed { .. } => "removed",
        }
    }
}

/// Deterministic in-memory object store.
#[derive(Debug, Clone, Default)]
pub struct ResourceStore {
    objects: BTreeMap<ResourceId, Arc<DynamicObject>>,
    by_gvk: BTreeMap<Gvk, BTreeSet<ResourceId>>,
    by_namespace: BTreeMap<Option<String>, BTreeSet<ResourceId>>,
    by_owner_uid: BTreeMap<String, BTreeSet<ResourceId>>,
    by_owner_ref: BTreeMap<OwnerRefKey, BTreeSet<ResourceId>>,
    generation: u64,
}

impl ResourceStore {
    /// Creates an empty store at generation zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces an object and returns the resulting event.
    ///
    /// Replacement always increments the generation; comparing object content
    /// is a later optimization so updates that only change status are visible.
    pub fn upsert(&mut self, object: DynamicObject) -> StoreEvent {
        let id = resource_id(&object);
        let existed = self.objects.contains_key(&id);
        if existed {
            self.remove_indexes(&id);
        }
        let object = Arc::new(object);
        self.index_insert(&id, &object);
        self.objects.insert(id.clone(), object);
        let generation = self.bump_generation();
        if existed {
            StoreEvent::Updated { id, generation }
        } else {
            StoreEvent::Added { id, generation }
        }
    }

    /// Removes an object, returning its removal event when present.
    pub fn remove(&mut self, id: &ResourceId) -> Option<StoreEvent> {
        if !self.objects.contains_key(id) {
            return None;
        }
        self.remove_indexes(id);
        let object = self.objects.remove(id)?;
        drop(object);
        let generation = self.bump_generation();
        Some(StoreEvent::Removed {
            id: id.clone(),
            generation,
        })
    }

    /// Returns an object by identity.
    #[must_use]
    pub fn get(&self, id: &ResourceId) -> Option<Arc<DynamicObject>> {
        self.objects.get(id).cloned()
    }

    /// Returns true when the store contains the identity.
    #[must_use]
    pub fn contains(&self, id: &ResourceId) -> bool {
        self.objects.contains_key(id)
    }

    /// Iterates over objects in `ResourceId` order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&ResourceId, &Arc<DynamicObject>)> {
        self.objects.iter()
    }

    /// Returns all objects in deterministic order.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<DynamicObject>> {
        self.objects.values().cloned().collect()
    }

    /// Returns all identities in deterministic order.
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = &ResourceId> {
        self.objects.keys()
    }

    /// Returns the current generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Increments and returns the generation.
    pub fn bump_generation(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    /// Number of stored objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// True when the store contains no objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Identities for one GVK.
    #[must_use]
    pub fn ids_by_gvk(&self, gvk: &Gvk) -> Vec<ResourceId> {
        self.by_gvk
            .get(gvk)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Identities in one namespace. `None` returns cluster-scoped objects.
    #[must_use]
    pub fn ids_in_namespace(&self, namespace: Option<&str>) -> Vec<ResourceId> {
        let key = namespace.map(ToOwned::to_owned);
        self.by_namespace
            .get(&key)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Identities owned by a Kubernetes UID.
    #[must_use]
    pub fn ids_owned_by_uid(&self, uid: &str) -> Vec<ResourceId> {
        self.by_owner_uid
            .get(uid)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Identities owned by an owner reference key.
    #[must_use]
    pub fn ids_owned_by_ref(&self, key: &OwnerRefKey) -> Vec<ResourceId> {
        self.by_owner_ref
            .get(key)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Objects carrying every selector label with an equal value.
    #[must_use]
    pub fn list_by_labels(&self, selector: &BTreeMap<String, String>) -> Vec<Arc<DynamicObject>> {
        self.objects
            .values()
            .filter(|object| matches_labels(object, selector))
            .cloned()
            .collect()
    }

    fn index_insert(&mut self, id: &ResourceId, object: &DynamicObject) {
        self.by_gvk.entry(id.gvk()).or_default().insert(id.clone());
        self.by_namespace
            .entry(id.namespace.clone())
            .or_default()
            .insert(id.clone());
        for uid in owner_uids(object) {
            self.by_owner_uid.entry(uid).or_default().insert(id.clone());
        }
        for key in owner_refs(object) {
            self.by_owner_ref.entry(key).or_default().insert(id.clone());
        }
    }

    fn remove_indexes(&mut self, id: &ResourceId) {
        remove_from_index(&mut self.by_gvk, &id.gvk(), id);
        remove_from_index(&mut self.by_namespace, &id.namespace, id);
        for uid in owner_uids_of(self, id) {
            remove_from_index(&mut self.by_owner_uid, &uid, id);
        }
        for key in owner_refs_of(self, id) {
            remove_from_index(&mut self.by_owner_ref, &key, id);
        }
    }
}

fn remove_from_index<K: Ord + Clone>(
    index: &mut BTreeMap<K, BTreeSet<ResourceId>>,
    key: &K,
    id: &ResourceId,
) {
    if let Some(ids) = index.get_mut(key) {
        ids.remove(id);
        if ids.is_empty() {
            index.remove(key);
        }
    }
}

fn owner_uids_of(store: &ResourceStore, id: &ResourceId) -> Vec<String> {
    store
        .objects
        .get(id)
        .map(|object| owner_uids(object))
        .unwrap_or_default()
}

fn owner_refs_of(store: &ResourceStore, id: &ResourceId) -> Vec<OwnerRefKey> {
    store
        .objects
        .get(id)
        .map(|object| owner_refs(object))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource::tests::test_object;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

    fn owned(
        api_version: &str,
        kind: &str,
        namespace: Option<&str>,
        name: &str,
        owner_uid: &str,
        owner_name: &str,
        labels: &[(&str, &str)],
    ) -> DynamicObject {
        let mut object = test_object(api_version, kind, namespace, name);
        object.metadata.uid = Some(format!("uid-{name}"));
        object.metadata.owner_references = Some(vec![OwnerReference {
            api_version: "serving.knative.dev/v1".to_owned(),
            kind: "Service".to_owned(),
            name: owner_name.to_owned(),
            uid: owner_uid.to_owned(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }]);
        object.metadata.labels = Some(
            labels
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        );
        object
    }

    #[test]
    fn upsert_add_update_remove_and_generations() {
        let mut store = ResourceStore::new();
        assert!(store.is_empty());

        let object = test_object("v1", "Pod", Some("default"), "web");
        let added = store.upsert(object);
        assert!(matches!(added, StoreEvent::Added { generation: 1, .. }));
        assert_eq!(store.len(), 1);
        assert_eq!(store.generation(), 1);

        let object = test_object("v1", "Pod", Some("default"), "web");
        let updated = store.upsert(object);
        assert!(matches!(updated, StoreEvent::Updated { generation: 2, .. }));
        assert_eq!(store.len(), 1);
        assert_eq!(store.generation(), 2);

        let id = resource_id(&test_object("v1", "Pod", Some("default"), "web"));
        let removed = store.remove(&id);
        assert!(matches!(
            removed,
            Some(StoreEvent::Removed { generation: 3, .. })
        ));
        assert!(store.is_empty());
        assert!(store.remove(&id).is_none());
        assert_eq!(store.generation(), 3);
    }

    #[test]
    fn stable_ids_keep_graph_position() {
        let mut first = test_object("v1", "Pod", Some("default"), "web");
        first.metadata.uid = Some("uid-one".to_owned());
        let mut second = test_object("v1", "Pod", Some("default"), "web");
        second.metadata.uid = Some("uid-two".to_owned());
        let mut store = ResourceStore::new();
        store.upsert(first);
        store.upsert(second);
        assert_eq!(store.len(), 1);
        assert_eq!(store.generation(), 2);
    }

    #[test]
    fn indexes_are_queryable_and_cleaned() {
        let mut store = ResourceStore::new();
        store.upsert(owned(
            "serving.knative.dev/v1",
            "Revision",
            Some("default"),
            "hello-00001",
            "uid-service",
            "hello",
            &[("app", "hello"), ("tier", "frontend")],
        ));
        store.upsert(test_object("v1", "Pod", Some("other"), "worker"));

        assert_eq!(
            store.ids_by_gvk(&Gvk::new("serving.knative.dev", "v1", "Revision")),
            vec![ResourceId::namespaced(
                "serving.knative.dev/v1",
                "Revision",
                "default",
                "hello-00001"
            )]
        );
        assert_eq!(store.ids_in_namespace(Some("other")).len(), 1);
        assert_eq!(store.ids_owned_by_uid("uid-service").len(), 1);
        assert_eq!(
            store
                .ids_owned_by_ref(&OwnerRefKey::new(
                    "serving.knative.dev/v1",
                    "Service",
                    "hello"
                ))
                .len(),
            1
        );

        let mut selector = BTreeMap::new();
        selector.insert("app".to_owned(), "hello".to_owned());
        assert_eq!(store.list_by_labels(&selector).len(), 1);

        let id = ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Revision",
            "default",
            "hello-00001",
        );
        store.remove(&id);
        assert!(store.ids_by_gvk(&id.gvk()).is_empty());
        assert!(store.ids_in_namespace(Some("default")).is_empty());
        assert!(store.ids_owned_by_uid("uid-service").is_empty());
    }

    #[test]
    fn iteration_is_deterministic() {
        let mut store = ResourceStore::new();
        for name in ["c", "a", "b"] {
            store.upsert(test_object("v1", "Pod", Some("default"), name));
        }
        let names: Vec<String> = store.iter().map(|(id, _)| id.name.clone()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        let reverse: Vec<String> = store.ids().rev().map(|id| id.name.clone()).collect();
        assert_eq!(reverse, vec!["c", "b", "a"]);
    }

    #[test]
    fn explicit_generation_bumps_are_monotonic() {
        let mut store = ResourceStore::new();
        assert_eq!(store.bump_generation(), 1);
        assert_eq!(store.bump_generation(), 2);
        assert_eq!(store.generation(), 2);
    }
}
