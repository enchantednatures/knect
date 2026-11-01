//! RabbitMQ operator `RabbitmqCluster` descriptions.

use kube::ResourceExt;
use kube::api::DynamicObject;

use super::project::{
    badges, first_condition_health, first_text, first_u64, replica_health, summary,
};
use super::{Description, OperatorMetadata};
use crate::graph::support::object_name;

/// Describes a `RabbitmqCluster`.
pub(crate) fn describe(object: &DynamicObject) -> Description {
    let replicas = first_u64(object, &[&["spec", "replicas"]]);
    let ready = first_u64(object, &[&["status", "readyReplicas"]]);
    let health = first_condition_health(object, &["AllReplicasReady", "ClusterAvailable", "Ready"])
        .unwrap_or_else(|| replica_health(ready, replicas));
    let queue = first_text(object, &[&["spec", "queue"], &["spec", "queueName"]])
        .or_else(|| object.labels().get("rabbitmq.com/queue").cloned());
    let exchange = first_text(object, &[&["spec", "exchange"], &["spec", "exchangeName"]])
        .or_else(|| object.labels().get("rabbitmq.com/exchange").cloned());
    let metadata = OperatorMetadata {
        replicas,
        ready_replicas: ready,
        queue,
        exchange,
        ..OperatorMetadata::default()
    };
    let name = object_name(object).unwrap_or("unnamed");
    Description {
        health,
        summary: summary("RabbitmqCluster", name, &metadata, health, "replicas"),
        badges: badges(health, &metadata, "replicas"),
        metadata,
    }
}
