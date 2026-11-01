//! ReadWriteOnce autoscaling conflict detection.
//!
//! A claim that only allows one writer cannot back a Knative Service that may
//! run more than one replica. The linter treats an absent, zero, or greater
//! than one `autoscaling.knative.dev/maxScale` as a conflict.

use knect_proto::ResourceId;
use knect_proto::storage::AccessMode;
use kube::api::DynamicObject;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::graph::support::field;
use crate::storage::{access_mode_label, is_read_write_once};

use super::owners::{owning_kservice, workload_revision};
use crate::store::ResourceStore;

use super::mounts::Mount;

/// Stable graph and storage warning code.
pub(crate) const CONFLICT_CODE: &str = "rwo_maxscale_conflict";
/// Stable node badge label.
pub(crate) const CONFLICT_BADGE: &str = "rwo-maxscale-conflict";

const MAX_SCALE_ANNOTATION: &str = "autoscaling.knative.dev/maxScale";

/// One autoscaling conflict discovered for a mounted claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Conflict {
    /// Workload mounting the claim.
    pub workload: ResourceId,
    /// Knative Service owning the workload.
    pub kservice: ResourceId,
    /// Read-write-once claim.
    pub pvc: ResourceId,
    /// Access mode that triggered the conflict.
    pub access_mode: AccessMode,
    /// Effective maximum scale, or `None` when the annotation is absent.
    pub max_scale: Option<u32>,
}

/// Finds every workload mount that conflicts with its owning Service's scale.
pub(crate) fn conflicts(
    store: &ResourceStore,
    access_modes: &BTreeMap<ResourceId, Vec<AccessMode>>,
    mounts: &[Mount],
) -> Vec<Conflict> {
    let mut seen: BTreeSet<(ResourceId, ResourceId)> = BTreeSet::new();
    let mut conflicts = Vec::new();

    for mount in mounts {
        let Some(modes) = access_modes.get(&mount.pvc) else {
            continue;
        };
        let Some(access_mode) = modes.iter().copied().find(|mode| is_read_write_once(*mode)) else {
            continue;
        };
        let Some(kservice) = owning_kservice(store, &mount.workload) else {
            continue;
        };
        let max_scale = effective_max_scale(store, &kservice, &mount.workload);
        if !max_scale_conflicts(max_scale) {
            continue;
        }
        if !seen.insert((mount.workload.clone(), mount.pvc.clone())) {
            continue;
        }
        conflicts.push(Conflict {
            workload: mount.workload.clone(),
            kservice,
            pvc: mount.pvc.clone(),
            access_mode,
            max_scale,
        });
    }
    conflicts.sort();
    conflicts
}

/// Returns true when the effective scale allows concurrent writers.
#[must_use]
pub(crate) fn max_scale_conflicts(max_scale: Option<u32>) -> bool {
    match max_scale {
        None | Some(0) => true,
        Some(scale) => scale > 1,
    }
}

/// Human-readable explanation for one conflict.
#[must_use]
pub(crate) fn conflict_reason(access_mode: AccessMode, max_scale: Option<u32>) -> String {
    let scale = match max_scale {
        None => "an unset maxScale".to_owned(),
        Some(scale) => format!("maxScale={scale}"),
    };
    format!(
        "{} claims cannot be mounted by multiple concurrent replicas ({scale})",
        access_mode_label(access_mode)
    )
}

fn effective_max_scale(
    store: &ResourceStore,
    kservice: &ResourceId,
    workload: &ResourceId,
) -> Option<u32> {
    let declared = store
        .get(kservice)
        .and_then(|object| max_scale_annotation(&object).map(ToOwned::to_owned));
    let declared = declared.or_else(|| {
        store
            .get(workload)
            .and_then(|object| max_scale_annotation(&object).map(ToOwned::to_owned))
    });
    let declared = declared.or_else(|| {
        workload_revision(store, workload)
            .and_then(|revision| max_scale_annotation(&revision).map(ToOwned::to_owned))
    });
    declared.and_then(|value| value.trim().parse::<u32>().ok())
}

fn max_scale_annotation(object: &DynamicObject) -> Option<&str> {
    let declared = object
        .metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(MAX_SCALE_ANNOTATION))
        .map(String::as_str);
    declared.or_else(|| {
        field(
            object,
            &[
                "spec",
                "template",
                "metadata",
                "annotations",
                MAX_SCALE_ANNOTATION,
            ],
        )
        .and_then(Value::as_str)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_scale_boundaries() {
        assert!(max_scale_conflicts(None));
        assert!(max_scale_conflicts(Some(0)));
        assert!(!max_scale_conflicts(Some(1)));
        assert!(max_scale_conflicts(Some(2)));
        assert!(max_scale_conflicts(Some(5)));
        assert!(conflict_reason(AccessMode::ReadWriteOnce, Some(5)).contains("maxScale=5"));
    }
}
