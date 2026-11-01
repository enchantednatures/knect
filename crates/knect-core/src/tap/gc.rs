//! Startup garbage collection for daemon-managed tap resources.

use std::collections::BTreeSet;

use knect_proto::Gvk;
use kube::api::DynamicObject;

use super::trigger::{
    MANAGED_BY_LABEL, MANAGED_BY_VALUE, TAP_ID_LABEL, TriggerCoordinates, delete_trigger_resources,
    secret_name_for, secret_resource, trigger_resource,
};
use crate::error::CoreError;
use crate::kube::ClusterReader;

/// Deletes daemon-managed Trigger/Secret objects whose tap is no longer live.
pub(crate) async fn gc_orphans(
    reader: &std::sync::Arc<dyn ClusterReader>,
    active_ids: &BTreeSet<String>,
) -> Result<Vec<String>, CoreError> {
    let triggers = reader.list(&trigger_resource(), None).await?;
    let secrets = reader.list(&secret_resource(), None).await?;
    let mut removed = BTreeSet::new();

    for object in &triggers {
        let Some(id) = managed_tap_id(object) else {
            continue;
        };
        if active_ids.contains(&id) {
            continue;
        }
        let namespace = object.metadata.namespace.clone().unwrap_or_default();
        let name = object.metadata.name.clone().unwrap_or_default();
        if namespace.is_empty() || name.is_empty() {
            continue;
        }
        let coordinates = TriggerCoordinates {
            namespace: namespace.clone(),
            trigger_name: name,
            secret_name: secret_name_for(&id),
        };
        delete_trigger_resources(reader, &coordinates).await?;
        removed.insert(id);
    }

    for object in &secrets {
        let Some(id) = managed_tap_id(object) else {
            continue;
        };
        if active_ids.contains(&id) || removed.contains(&id) {
            continue;
        }
        let namespace = object.metadata.namespace.clone().unwrap_or_default();
        let name = object.metadata.name.clone().unwrap_or_default();
        if namespace.is_empty() || name.is_empty() {
            continue;
        }
        reader
            .delete(&Gvk::new("", "v1", "Secret"), Some(&namespace), &name)
            .await?;
        removed.insert(id);
    }

    Ok(removed.into_iter().collect())
}

fn managed_tap_id(object: &DynamicObject) -> Option<String> {
    let labels = object.metadata.labels.as_ref()?;
    if labels.get(MANAGED_BY_LABEL).map(String::as_str) != Some(MANAGED_BY_VALUE) {
        return None;
    }
    labels.get(TAP_ID_LABEL).cloned().or_else(|| {
        object
            .metadata
            .annotations
            .as_ref()?
            .get(TAP_ID_LABEL)
            .cloned()
    })
}
