//! Workload volume mount discovery.

use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;
use serde_json::Value;

use crate::graph::support::field;
use crate::store::ResourceStore;

/// One persistent volume claim mounted by a workload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Mount {
    /// Workload that declares the volume.
    pub workload: ResourceId,
    /// Claim the volume references.
    pub pvc: ResourceId,
    /// Volume name inside the workload spec.
    pub volume: String,
    /// Mount path inside the first matching container.
    pub mount_path: Option<String>,
}

pub(crate) fn collect_mounts(store: &ResourceStore) -> Vec<Mount> {
    let mut workload_ids = store.ids_by_gvk(&Gvk::new("serving.knative.dev", "v1", "Revision"));
    workload_ids.extend(store.ids_by_gvk(&Gvk::new("apps", "v1", "Deployment")));
    workload_ids.extend(store.ids_by_gvk(&Gvk::new("", "v1", "Pod")));
    workload_ids.sort();

    let mut mounts = Vec::new();
    for workload_id in workload_ids {
        let Some(object) = store.get(&workload_id) else {
            continue;
        };
        let volumes = volume_slice(&object, &workload_id.kind);
        let containers = container_slice(&object, &workload_id.kind);
        for volume in volumes {
            let Some(volume_name) = volume.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(claim_name) = volume
                .get("persistentVolumeClaim")
                .and_then(|claim| claim.get("claimName"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            mounts.push(Mount {
                pvc: ResourceId::namespaced(
                    "v1",
                    "PersistentVolumeClaim",
                    workload_id.namespace.clone().unwrap_or_default(),
                    claim_name,
                ),
                workload: workload_id.clone(),
                volume: volume_name.to_owned(),
                mount_path: mount_path(containers, volume_name),
            });
        }
    }
    mounts.sort_by(|left, right| {
        (&left.workload, &left.pvc, &left.volume).cmp(&(&right.workload, &right.pvc, &right.volume))
    });
    mounts.dedup_by(|left, right| {
        left.workload == right.workload && left.pvc == right.pvc && left.volume == right.volume
    });
    mounts
}

fn volume_slice<'a>(object: &'a DynamicObject, kind: &str) -> &'a [Value] {
    let path: &[&str] = match kind {
        "Deployment" => &["spec", "template", "spec", "volumes"],
        _ => &["spec", "volumes"],
    };
    field(object, path)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn container_slice<'a>(object: &'a DynamicObject, kind: &str) -> &'a [Value] {
    let path: &[&str] = match kind {
        "Deployment" => &["spec", "template", "spec", "containers"],
        _ => &["spec", "containers"],
    };
    field(object, path)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn mount_path(containers: &[Value], volume_name: &str) -> Option<String> {
    containers.iter().find_map(|container| {
        container
            .get("volumeMounts")?
            .as_array()?
            .iter()
            .find(|mount| mount.get("name").and_then(Value::as_str) == Some(volume_name))
            .and_then(|mount| mount.get("mountPath"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}
