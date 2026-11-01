//! Backing operator CRD boundary.
//!
//! Operator CRDs are matched by API group and kind, never by version, so a
//! cluster serving `v1alpha1`, `v1beta2`, or `v1` exposes the same backing
//! service and topic nodes. Each family handler returns a [`Description`] that
//! the operators correlator projects into a `BackingService` or `Topic` node.

pub mod cnpg;
pub mod edges;
pub mod kafka;
pub mod rabbitmq;
pub mod redis;
pub mod redpanda;

mod project;
mod resolve;

pub use knect_proto::ResourceId;
pub use knect_proto::graph::{Health, NodeCategory};

use knect_proto::graph::Badge;
use kube::api::DynamicObject;

use crate::correlate::CorrelationContext;
use crate::error::CoreError;
use crate::store::{ResourceStore, resource_id};

pub use self::edges::add_edges;

/// Lists backing operator resources visible in the current store.
pub trait OperatorCatalog: Send + Sync {
    /// Returns backing-service identities.
    fn backing_services(&self, ctx: &CorrelationContext) -> Result<Vec<ResourceId>, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(catalog: std::sync::Arc<dyn OperatorCatalog>) -> std::sync::Arc<dyn OperatorCatalog> {
    catalog
}

/// Supported backing operator families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OperatorFamily {
    /// Strimzi `kafka.strimzi.io` `Kafka`.
    StrimziKafka,
    /// Strimzi `kafka.strimzi.io` `KafkaTopic`.
    StrimziTopic,
    /// Redpanda `cluster.redpanda.com` `Cluster`.
    RedpandaCluster,
    /// Redpanda `cluster.redpanda.com` `Topic`.
    RedpandaTopic,
    /// RabbitMQ `rabbitmq.com` `RabbitmqCluster`.
    RabbitmqCluster,
    /// Dragonfly `dragonflydb.io` `Dragonfly`.
    Dragonfly,
    /// Redis operator `redis.redis.opstreelabs.in` `RedisCluster`.
    RedisCluster,
    /// CloudNativePG `postgresql.cnpg.io` `Cluster`.
    CnpgCluster,
}

impl OperatorFamily {
    /// Matches a resource identity by group and kind, ignoring the version.
    #[must_use]
    pub fn from_id(id: &ResourceId) -> Option<Self> {
        let (group, _version) = id.api_version.split_once('/')?;
        match (group, id.kind.as_str()) {
            ("kafka.strimzi.io", "Kafka") => Some(Self::StrimziKafka),
            ("kafka.strimzi.io", "KafkaTopic") => Some(Self::StrimziTopic),
            ("cluster.redpanda.com", "Cluster") => Some(Self::RedpandaCluster),
            ("cluster.redpanda.com", "Topic") => Some(Self::RedpandaTopic),
            ("rabbitmq.com", "RabbitmqCluster") => Some(Self::RabbitmqCluster),
            ("dragonflydb.io", "Dragonfly") => Some(Self::Dragonfly),
            ("redis.redis.opstreelabs.in", "RedisCluster") => Some(Self::RedisCluster),
            ("postgresql.cnpg.io", "Cluster") => Some(Self::CnpgCluster),
            _ => None,
        }
    }

    /// Node category used for this family.
    #[must_use]
    pub const fn category(self) -> NodeCategory {
        match self {
            Self::StrimziTopic | Self::RedpandaTopic => NodeCategory::Topic,
            Self::StrimziKafka
            | Self::RedpandaCluster
            | Self::RabbitmqCluster
            | Self::Dragonfly
            | Self::RedisCluster
            | Self::CnpgCluster => NodeCategory::BackingService,
        }
    }

    /// True for topic resources.
    #[must_use]
    pub const fn is_topic(self) -> bool {
        matches!(self, Self::StrimziTopic | Self::RedpandaTopic)
    }

    /// True for every cluster-scoped backing family (anything but a topic).
    #[must_use]
    pub const fn is_cluster(self) -> bool {
        !self.is_topic()
    }

    /// True for cluster families that can back a Knative Broker.
    #[must_use]
    pub const fn is_broker_cluster(self) -> bool {
        matches!(
            self,
            Self::StrimziKafka | Self::RedpandaCluster | Self::RabbitmqCluster
        )
    }

    /// Human-readable family label used in summaries and edge metadata.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::StrimziKafka => "Kafka",
            Self::StrimziTopic => "KafkaTopic",
            Self::RedpandaCluster => "Redpanda Cluster",
            Self::RedpandaTopic => "Redpanda Topic",
            Self::RabbitmqCluster => "RabbitmqCluster",
            Self::Dragonfly => "Dragonfly",
            Self::RedisCluster => "RedisCluster",
            Self::CnpgCluster => "CloudNativePG Cluster",
        }
    }
}

/// Capacity and status metadata extracted from an operator CRD.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorMetadata {
    /// Desired replicas or instances.
    pub replicas: Option<u64>,
    /// Ready replicas or instances.
    pub ready_replicas: Option<u64>,
    /// Topic partition count.
    pub partitions: Option<u64>,
    /// Topic replication factor.
    pub replication_factor: Option<u64>,
    /// Queue name exposed by the CRD.
    pub queue: Option<String>,
    /// Exchange name exposed by the CRD.
    pub exchange: Option<String>,
    /// Free-form operator phase.
    pub phase: Option<String>,
}

/// A fully described backing operator resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorNode {
    /// Canonical identity.
    pub id: ResourceId,
    /// Matched operator family.
    pub family: OperatorFamily,
    /// `BackingService` or `Topic`.
    pub category: NodeCategory,
    /// Roll-up health.
    pub health: Health,
    /// Deterministic one-line summary.
    pub summary: String,
    /// UI badges.
    pub badges: Vec<Badge>,
    /// Capacity metadata.
    pub metadata: OperatorMetadata,
}

/// Internal description returned by a family handler.
pub(crate) struct Description {
    /// Roll-up health.
    pub health: Health,
    /// Deterministic one-line summary.
    pub summary: String,
    /// UI badges.
    pub badges: Vec<Badge>,
    /// Capacity metadata.
    pub metadata: OperatorMetadata,
}

/// Describes every supported operator resource in the store.
#[must_use]
pub fn describe_all(store: &ResourceStore) -> Vec<OperatorNode> {
    let mut nodes: Vec<OperatorNode> = store
        .iter()
        .filter_map(|(_, object)| describe(object))
        .collect();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    nodes
}

/// Describes one operator resource, or `None` for an unsupported object.
#[must_use]
pub fn describe(object: &DynamicObject) -> Option<OperatorNode> {
    let id = resource_id(object);
    if id.name.is_empty() {
        return None;
    }
    let family = OperatorFamily::from_id(&id)?;
    let description = match family {
        OperatorFamily::StrimziKafka => kafka::describe_kafka(object),
        OperatorFamily::StrimziTopic => kafka::describe_topic(object),
        OperatorFamily::RedpandaCluster => redpanda::describe_cluster(object),
        OperatorFamily::RedpandaTopic => redpanda::describe_topic(object),
        OperatorFamily::RabbitmqCluster => rabbitmq::describe(object),
        OperatorFamily::Dragonfly => redis::describe_dragonfly(object),
        OperatorFamily::RedisCluster => redis::describe_redis_cluster(object),
        OperatorFamily::CnpgCluster => cnpg::describe(object),
    };
    Some(OperatorNode {
        id,
        family,
        category: family.category(),
        health: description.health,
        summary: description.summary,
        badges: description.badges,
        metadata: description.metadata,
    })
}
