//! CloudNativePG `Cluster` descriptions.

use kube::api::DynamicObject;

use super::project::{
    badges, first_condition_health, first_text, first_u64, phase_health, replica_health, summary,
};
use super::{Description, OperatorMetadata};
use crate::graph::support::object_name;

/// Describes a CloudNativePG `Cluster`.
pub(crate) fn describe(object: &DynamicObject) -> Description {
    let instances = first_u64(object, &[&["spec", "instances"], &["status", "instances"]]);
    let ready = first_u64(object, &[&["status", "readyInstances"]]);
    let phase = first_text(object, &[&["status", "phase"]]);
    let health = first_condition_health(object, &["Ready", "Healthy"])
        .or_else(|| phase.as_deref().map(phase_health))
        .unwrap_or_else(|| replica_health(ready, instances));
    let metadata = OperatorMetadata {
        replicas: instances,
        ready_replicas: ready,
        phase,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary(
            "CloudNativePG Cluster",
            name,
            &metadata,
            health,
            "instances",
        ),
        badges: badges(health, &metadata, "instances"),
        metadata,
    }
}
