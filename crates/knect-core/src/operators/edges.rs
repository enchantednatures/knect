//! Edges between operator CRDs and the resources that consume them.
//!
//! Only existing nodes receive edges: a Broker must already be in the graph, a
//! Service node must have been projected by another correlator, and every
//! backing target must be one of the operator nodes described in this stage.
//! Missing CRDs therefore produce no node and no edge.

use knect_proto::graph::{Edge, EdgeKind};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::json;

use crate::graph::GraphBuilder;
use crate::graph::support::{
    add_unique_edge, insert_metadata, insert_text_metadata, new_edge, text_field,
};
use crate::operators::OperatorNode;
use crate::store::ResourceStore;

use super::resolve::{
    BackingIndex, ClusterIndex, TopicIndex, broker_class, broker_cluster_compatible,
    broker_service_hosts, is_broker_id, service_ids, topic_cluster_compatible, topic_cluster_name,
};

/// Adds `BackedBy` and `ConsumesFrom` edges for operator CRDs.
pub fn add_edges(store: &ResourceStore, graph: &mut GraphBuilder, nodes: &[OperatorNode]) {
    let clusters = ClusterIndex::new(nodes);
    let topics = TopicIndex::new(nodes);
    let services = service_ids(store);
    let backing = BackingIndex::new(clusters, &services);
    add_service_edges(graph, &backing, &services);
    add_topic_edges(store, graph, &topics, &backing);
    add_broker_edges(store, graph, &backing, &topics);
}

fn add_service_edges(
    graph: &mut GraphBuilder,
    backing: &BackingIndex<'_>,
    services: &[ResourceId],
) {
    for service_id in services {
        let Some(cluster) = backing.for_service(service_id.namespace.as_deref(), &service_id.name)
        else {
            continue;
        };
        let Some(node) = graph.node(service_id) else {
            continue;
        };
        if !matches!(
            node.category,
            NodeCategory::Service | NodeCategory::Endpoints
        ) {
            continue;
        }
        let mut edge = new_edge(EdgeKind::BackedBy, service_id, &cluster.id);
        edge.health = cluster.health;
        insert_text_metadata(&mut edge, "family", cluster.family.label());
        add_unique_edge(graph, edge, Some(&service_id.name));
    }
}

fn add_topic_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    topics: &TopicIndex<'_>,
    backing: &BackingIndex<'_>,
) {
    for topic in topics.iter() {
        if !graph.contains_node(&topic.id) {
            continue;
        }
        let Some(object) = store.get(&topic.id) else {
            continue;
        };
        let Some(cluster_name) = topic_cluster_name(&object) else {
            continue;
        };
        let Some(cluster) = backing
            .clusters
            .by_name(topic.id.namespace.as_deref(), &cluster_name)
        else {
            continue;
        };
        if !topic_cluster_compatible(topic.family, cluster.family) {
            continue;
        }
        let mut edge = new_edge(EdgeKind::BackedBy, &topic.id, &cluster.id);
        edge.health = cluster.health;
        insert_text_metadata(&mut edge, "family", topic.family.label());
        insert_text_metadata(&mut edge, "cluster_family", cluster.family.label());
        add_topic_metadata(&mut edge, topic);
        add_unique_edge(graph, edge, None);
    }
}

fn add_broker_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    backing: &BackingIndex<'_>,
    topics: &TopicIndex<'_>,
) {
    for (broker_id, object) in store.iter() {
        if !is_broker_id(broker_id) || !graph.contains_node(broker_id) {
            continue;
        }
        let object = object.as_ref();
        let class = broker_class(object);
        let lower = class.to_ascii_lowercase();
        let kafka = lower.contains("kafka");
        let rabbitmq = lower.contains("rabbitmq");
        if !kafka && !rabbitmq {
            continue;
        }

        let hosts = broker_service_hosts(object, broker_id.namespace.as_deref());
        if let Some(cluster) = hosts
            .iter()
            .filter_map(|(namespace, name)| backing.for_service(namespace.as_deref(), name))
            .find(|cluster| broker_cluster_compatible(&lower, cluster.family))
        {
            add_backed_by(graph, broker_id, cluster, class);
        }

        if kafka
            && let Some(topic_name) =
                text_field(object, &["status", "annotations", "default.topic"])
            && let Some(topic) = topics.by_name(broker_id.namespace.as_deref(), topic_name)
        {
            add_consumes_from(graph, broker_id, topic, class);
        }
    }
}

fn add_backed_by(
    graph: &mut GraphBuilder,
    broker_id: &ResourceId,
    cluster: &OperatorNode,
    class: &str,
) {
    let mut edge = new_edge(EdgeKind::BackedBy, broker_id, &cluster.id);
    edge.health = cluster.health;
    insert_text_metadata(&mut edge, "class", class);
    insert_text_metadata(&mut edge, "family", cluster.family.label());
    if let Some(replicas) = cluster.metadata.replicas {
        insert_metadata(&mut edge, "replicas", json!(replicas));
    }
    if let Some(ready) = cluster.metadata.ready_replicas {
        insert_metadata(&mut edge, "ready_replicas", json!(ready));
    }
    add_unique_edge(graph, edge, Some(class));
}

fn add_consumes_from(
    graph: &mut GraphBuilder,
    broker_id: &ResourceId,
    topic: &OperatorNode,
    class: &str,
) {
    let mut edge = new_edge(EdgeKind::ConsumesFrom, broker_id, &topic.id);
    edge.health = topic.health;
    insert_text_metadata(&mut edge, "class", class);
    insert_text_metadata(&mut edge, "family", topic.family.label());
    add_topic_metadata(&mut edge, topic);
    add_unique_edge(graph, edge, Some(class));
}

fn add_topic_metadata(edge: &mut Edge, topic: &OperatorNode) {
    if let Some(partitions) = topic.metadata.partitions {
        insert_metadata(edge, "partitions", json!(partitions));
    }
    if let Some(factor) = topic.metadata.replication_factor {
        insert_metadata(edge, "replication_factor", json!(factor));
    }
    if let Some(queue) = &topic.metadata.queue {
        insert_text_metadata(edge, "queue", queue);
    }
    if let Some(exchange) = &topic.metadata.exchange {
        insert_text_metadata(edge, "exchange", exchange);
    }
}
