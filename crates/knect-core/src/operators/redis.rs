//! Dragonfly and Redis operator descriptions.

use kube::api::DynamicObject;

use super::project::{
    badges, first_condition_health, first_text, first_u64, phase_health, replica_health, summary,
};
use super::{Description, OperatorMetadata};
use crate::graph::support::object_name;

/// Describes a Dragonfly instance.
pub(crate) fn describe_dragonfly(object: &DynamicObject) -> Description {
    let replicas = first_u64(object, &[&["spec", "replicas"]]);
    let ready = first_u64(object, &[&["status", "readyReplicas"]]);
    let phase = first_text(object, &[&["status", "phase"]]);
    let health = first_condition_health(object, &["Ready", "Available", "Healthy"])
        .or_else(|| phase.as_deref().map(phase_health))
        .unwrap_or_else(|| replica_health(ready, replicas));
    let metadata = OperatorMetadata {
        replicas,
        ready_replicas: ready,
        phase,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary("Dragonfly", name, &metadata, health, "replicas"),
        badges: badges(health, &metadata, "replicas"),
        metadata,
    }
}

/// Describes a Redis operator `RedisCluster`.
pub(crate) fn describe_redis_cluster(object: &DynamicObject) -> Description {
    let replicas = first_u64(
        object,
        &[
            &["spec", "replicas"],
            &["spec", "size"],
            &["spec", "kubernetesConfig", "replicas"],
        ],
    );
    let ready = first_u64(object, &[&["status", "readyReplicas"]]);
    let phase = first_text(object, &[&["status", "phase"]]);
    let health = first_condition_health(object, &["Ready", "Healthy"])
        .or_else(|| phase.as_deref().map(phase_health))
        .unwrap_or_else(|| replica_health(ready, replicas));
    let metadata = OperatorMetadata {
        replicas,
        ready_replicas: ready,
        phase,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary("RedisCluster", name, &metadata, health, "replicas"),
        badges: badges(health, &metadata, "replicas"),
        metadata,
    }
}
