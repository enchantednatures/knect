//! Strimzi `Kafka` and `KafkaTopic` descriptions.

use knect_proto::graph::Health;
use kube::api::DynamicObject;

use super::project::{badges, first_condition_health, first_u64, replica_health, summary};
use super::{Description, OperatorMetadata};
use crate::graph::support::object_name;

/// Describes a Strimzi `Kafka` cluster.
pub(crate) fn describe_kafka(object: &DynamicObject) -> Description {
    let replicas = first_u64(
        object,
        &[&["spec", "kafka", "replicas"], &["spec", "replicas"]],
    );
    let ready = first_u64(object, &[&["status", "readyReplicas"]]);
    let health = first_condition_health(object, &["Ready"])
        .unwrap_or_else(|| replica_health(ready, replicas));
    let metadata = OperatorMetadata {
        replicas,
        ready_replicas: ready,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary("Kafka", name, &metadata, health, "replicas"),
        badges: badges(health, &metadata, "replicas"),
        metadata,
    }
}

/// Describes a Strimzi `KafkaTopic`.
pub(crate) fn describe_topic(object: &DynamicObject) -> Description {
    let partitions = first_u64(object, &[&["spec", "partitions"]]);
    let replication_factor = first_u64(
        object,
        &[&["spec", "replicas"], &["spec", "replicationFactor"]],
    );
    let health = first_condition_health(object, &["Ready"]).unwrap_or(Health::Unknown);
    let metadata = OperatorMetadata {
        partitions,
        replication_factor,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary("KafkaTopic", name, &metadata, health, "replicas"),
        badges: badges(health, &metadata, "replicas"),
        metadata,
    }
}
