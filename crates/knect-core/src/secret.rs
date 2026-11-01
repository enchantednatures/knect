//! In-memory registry for sanitized Secret values.
//!
//! Values are keyed by [`ResourceId`] and never leave the process. The registry
//! intentionally implements neither `Serialize` nor `Deserialize`, and its
//! `Debug` output contains only identities and value counts.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use knect_proto::ResourceId;

use crate::kube::SecretMaterial;

/// Thread-safe registry of Secret material observed by watchers.
#[derive(Clone, Default)]
pub struct SecretMaterialRegistry {
    entries: Arc<RwLock<BTreeMap<ResourceId, SecretMaterial>>>,
}

impl SecretMaterialRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces material for a resource.
    pub fn insert(&self, id: ResourceId, material: SecretMaterial) {
        self.write().insert(id, material);
    }

    /// Removes material for a resource, returning it when present.
    pub fn remove(&self, id: &ResourceId) -> Option<SecretMaterial> {
        self.write().remove(id)
    }

    /// Returns a clone of the material for a resource.
    #[must_use]
    pub fn get(&self, id: &ResourceId) -> Option<SecretMaterial> {
        self.read().get(id).cloned()
    }

    /// Number of tracked Secrets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// True when no Secret material is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Drops every tracked Secret.
    pub fn clear(&self) {
        self.write().clear();
    }

    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<ResourceId, SecretMaterial>> {
        match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<ResourceId, SecretMaterial>> {
        match self.entries.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl fmt::Debug for SecretMaterialRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretMaterialRegistry")
            .field("entries", &self.len())
            .field("values", &knect_proto::redaction::REDACTED)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> ResourceId {
        ResourceId::namespaced("v1", "Secret", "default", name)
    }

    fn material() -> SecretMaterial {
        let mut material = SecretMaterial::default();
        material.insert("data", "password", "hunter2-knect-sentinel");
        material
    }

    #[test]
    fn insert_get_remove_and_clear() {
        let registry = SecretMaterialRegistry::new();
        assert!(registry.is_empty());
        registry.insert(id("credentials"), material());
        assert_eq!(registry.len(), 1);
        assert!(registry.get(&id("credentials")).is_some());

        let removed = registry.remove(&id("credentials"));
        assert!(removed.is_some());
        assert!(registry.is_empty());
        assert!(registry.get(&id("credentials")).is_none());

        registry.insert(id("one"), material());
        registry.insert(id("two"), material());
        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    fn debug_redacts_values() {
        let registry = SecretMaterialRegistry::new();
        registry.insert(id("credentials"), material());
        let debug = format!("{registry:?}");
        assert!(!debug.contains("hunter2-knect-sentinel"));
        assert!(debug.contains("entries"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn registry_is_send_sync_and_shareable() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SecretMaterialRegistry>();
        let registry = Arc::new(SecretMaterialRegistry::new());
        let clone = Arc::clone(&registry);
        registry.insert(id("one"), material());
        assert!(clone.get(&id("one")).is_some());
    }
}
