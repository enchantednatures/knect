//! Knative Eventing correlation.
//!
//! Builds Broker, Trigger, Source, SinkBinding, Subscription, and Channel nodes
//! with delivery, filter, backing, and dead-letter edges. Broker classes that
//! do not use a Knative Channel never get a synthetic Channel node.

use knect_proto::graph::{Badge, EdgeKind, Health, Severity};
use knect_proto::{NodeCategory, ResourceId};
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::{Value, json};

pub use crate::graph::edge_id;

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, field, insert_metadata, insert_text_metadata, namespaced, new_edge,
    object_name, ready_health, reference_id, resource_node, text_field, warning,
};
use crate::store::{ResourceStore, resource_id};

const EVENTING_API: &str = "eventing.knative.dev/v1";
const MESSAGING_GROUP: &str = "messaging.knative.dev";
const BROKER_CLASS_ANNOTATION: &str = "eventing.knative.dev/broker.class";
const DEFAULT_BROKER_CLASS: &str = "MTChannelBasedBroker";

/// Correlates Knative Eventing resources into the graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct EventingCorrelator;

impl Correlator for EventingCorrelator {
    fn name(&self) -> &'static str {
        "eventing"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let store = &ctx.store;
        let brokers = ids_in_group(store, "eventing.knative.dev", "Broker");
        let triggers = ids_in_group(store, "eventing.knative.dev", "Trigger");
        let subscriptions = ids_in_group(store, MESSAGING_GROUP, "Subscription");
        let channels = channel_ids(store);
        let sources = source_ids(store);

        add_broker_nodes(store, graph, &brokers);
        add_trigger_nodes(store, graph, &triggers);
        add_channel_nodes(store, graph, &channels);
        add_source_nodes(store, graph, &sources);
        add_subscription_nodes(store, graph, &subscriptions);

        add_broker_backing_edges(store, graph, &brokers);
        add_trigger_edges(store, graph, &triggers);
        add_source_edges(store, graph, &sources);
        add_subscription_edges(store, graph, &subscriptions);
        add_channel_backing_edges(store, graph, &channels);

        Ok(())
    }
}

fn ids_in_group(store: &ResourceStore, group: &str, kind: &str) -> Vec<ResourceId> {
    store
        .iter()
        .filter(|(id, _)| api_group(&id.api_version) == group && id.kind == kind)
        .map(|(id, _)| id.clone())
        .collect()
}

fn channel_ids(store: &ResourceStore) -> Vec<ResourceId> {
    store
        .iter()
        .filter(|(id, _)| {
            api_group(&id.api_version) == MESSAGING_GROUP && id.kind.ends_with("Channel")
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn source_ids(store: &ResourceStore) -> Vec<ResourceId> {
    store
        .iter()
        .filter(|(id, _)| id.api_version.starts_with("sources.knative.dev/"))
        .map(|(id, _)| id.clone())
        .collect()
}

fn api_group(api_version: &str) -> &str {
    api_version
        .split_once('/')
        .map_or("", |(group, _version)| group)
}

fn push_node(graph: &mut GraphBuilder, object: &DynamicObject, spec: NodeSpec) {
    if object_name(object).is_none() {
        let id = resource_id(object);
        graph.add_warning(warning(
            "malformed_object",
            format!("{id} has no metadata.name"),
            Some(id.clone()),
        ));
        return;
    }
    graph.add_node(resource_node(object, spec));
}

fn add_broker_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let class = broker_class(&object);
        let address = text_field(&object, &["status", "address", "url"]).unwrap_or("");
        let summary = if address.is_empty() {
            class.to_owned()
        } else {
            format!("{class} {address}")
        };
        push_node(
            graph,
            &object,
            NodeSpec {
                category: NodeCategory::Broker,
                health: ready_health(&object, Health::Unknown),
                summary,
                badges: Vec::new(),
            },
        );
    }
}

fn add_trigger_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let (_, filter_count) = trigger_filters(&object);
        let subscriber = subscriber_summary(&object);
        let mut badges = Vec::new();
        if filter_count > 0 {
            badges.push(Badge {
                label: "filtered".to_owned(),
                severity: Severity::Info,
                tooltip: Some(format!("{filter_count} filter attributes")),
            });
        }
        push_node(
            graph,
            &object,
            NodeSpec {
                category: NodeCategory::Trigger,
                health: ready_health(&object, Health::Unknown),
                summary: format!("{subscriber} ({filter_count} filters)"),
                badges,
            },
        );
    }
}

fn add_channel_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let summary = text_field(&object, &["status", "address", "url"])
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "no address".to_owned());
        push_node(
            graph,
            &object,
            NodeSpec {
                category: NodeCategory::Channel,
                health: ready_health(&object, Health::Unknown),
                summary,
                badges: Vec::new(),
            },
        );
    }
}

fn add_source_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let category = if id.kind == "SinkBinding" {
            NodeCategory::SinkBinding
        } else {
            NodeCategory::Source
        };
        push_node(
            graph,
            &object,
            NodeSpec {
                category,
                health: ready_health(&object, Health::Unknown),
                summary: sink_summary(&object),
                badges: Vec::new(),
            },
        );
    }
}

fn add_subscription_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let channel = field(&object, &["spec", "channel"])
            .and_then(Value::as_object)
            .and_then(|channel| channel.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("no channel");
        let subscriber = subscriber_summary(&object);
        push_node(
            graph,
            &object,
            NodeSpec {
                category: NodeCategory::Subscription,
                health: ready_health(&object, Health::Unknown),
                summary: format!("{channel} -> {subscriber}"),
                badges: Vec::new(),
            },
        );
    }
}

fn add_broker_backing_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    brokers: &[ResourceId],
) {
    let channels = channel_ids(store);
    for broker_id in brokers {
        let Some(broker) = store.get(broker_id) else {
            continue;
        };
        let class = broker_class(&broker);
        if !class.eq_ignore_ascii_case(DEFAULT_BROKER_CLASS) {
            add_class_backing_edge(graph, broker_id, &broker);
            continue;
        }
        if let Some(channel_id) = broker_channel(graph, &broker, &channels) {
            let mut edge = new_edge(EdgeKind::BackedBy, broker_id, &channel_id);
            insert_text_metadata(&mut edge, "class", class);
            add_unique_edge(graph, edge, None);
            continue;
        }
        graph.add_warning(warning(
            "missing_channel",
            format!(
                "Broker {} has no correlated backing Channel",
                broker_id.name
            ),
            Some(broker_id.clone()),
        ));
    }
}

fn add_class_backing_edge(
    graph: &mut GraphBuilder,
    broker_id: &ResourceId,
    broker: &DynamicObject,
) {
    let Some(config) = field(broker, &["spec", "config"]) else {
        return;
    };
    let Some(target) = reference_id(config, broker_id.namespace.as_deref()) else {
        return;
    };
    let is_backing = graph
        .node(&target)
        .is_some_and(|node| node.category == NodeCategory::BackingService);
    if !is_backing {
        return;
    }
    let mut edge = new_edge(EdgeKind::BackedBy, broker_id, &target);
    insert_text_metadata(&mut edge, "class", broker_class(broker));
    add_unique_edge(graph, edge, None);
}

fn broker_channel(
    graph: &GraphBuilder,
    broker: &DynamicObject,
    channels: &[ResourceId],
) -> Option<ResourceId> {
    let broker_id = resource_id(broker);
    let channel_name = text_field(
        broker,
        &["status", "annotations", "knative.dev/channelName"],
    );
    if let Some(name) = channel_name {
        let namespace = text_field(
            broker,
            &["status", "annotations", "knative.dev/channelNamespace"],
        )
        .or(broker_id.namespace.as_deref());
        let api_version = text_field(
            broker,
            &["status", "annotations", "knative.dev/channelAPIVersion"],
        )
        .unwrap_or("messaging.knative.dev/v1");
        let kind = text_field(
            broker,
            &["status", "annotations", "knative.dev/channelKind"],
        )
        .unwrap_or("InMemoryChannel");
        let candidate = namespaced(api_version, kind, namespace, name);
        if graph.contains_node(&candidate) {
            return Some(candidate);
        }
    }
    let child_name = format!("{}-kne-trigger", broker_id.name);
    channels
        .iter()
        .find(|channel| channel.name == child_name && channel.namespace == broker_id.namespace)
        .cloned()
}

fn add_trigger_edges(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for trigger_id in ids {
        let Some(trigger) = store.get(trigger_id) else {
            continue;
        };
        let (filter, filter_count) = trigger_filters(&trigger);
        let subscriber_ref = field(&trigger, &["spec", "subscriber", "ref"]);
        let subscriber_target = subscriber_ref
            .and_then(|reference| reference_id(reference, trigger_id.namespace.as_deref()))
            .filter(|id| graph.contains_node(id));
        let subscriber_uri = subscriber_uri(&trigger);

        match trigger_broker_id(graph, &trigger, trigger_id) {
            Some(broker_id) => {
                let mut edge = new_edge(EdgeKind::DeliversTo, &broker_id, trigger_id);
                if let Some(filter) = filter {
                    insert_metadata(&mut edge, "filter", filter);
                }
                if filter_count > 0 {
                    insert_metadata(&mut edge, "filter_count", json!(filter_count));
                }
                if subscriber_target.is_none()
                    && let Some(uri) = subscriber_uri
                {
                    insert_text_metadata(&mut edge, "subscriber_uri", uri);
                }
                add_unique_edge(graph, edge, None);
            }
            None => graph.add_warning(warning(
                "missing_broker",
                format!("Trigger {} has no correlated Broker", trigger_id.name),
                Some(trigger_id.clone()),
            )),
        }

        add_trigger_subscriber(graph, &trigger, &subscriber_target);
        add_trigger_dead_letter(graph, &trigger);
    }
}

fn add_trigger_subscriber(
    graph: &mut GraphBuilder,
    trigger: &DynamicObject,
    subscriber_target: &Option<ResourceId>,
) {
    let trigger_id = resource_id(trigger);
    if let Some(subscriber_id) = subscriber_target {
        let mut edge = new_edge(EdgeKind::DeliversTo, &trigger_id, subscriber_id);
        if let Some(uri) = subscriber_uri(trigger) {
            insert_text_metadata(&mut edge, "uri", uri);
        }
        add_unique_edge(graph, edge, None);
        return;
    }
    let has_ref = field(trigger, &["spec", "subscriber", "ref"]).is_some();
    if has_ref {
        graph.add_warning(warning(
            "missing_subscriber",
            format!(
                "Trigger {} subscriber reference is not in the topology",
                trigger_id.name
            ),
            Some(trigger_id.clone()),
        ));
    }
}

fn add_trigger_dead_letter(graph: &mut GraphBuilder, trigger: &DynamicObject) {
    let trigger_id = resource_id(trigger);
    let Some(sink) = field(trigger, &["spec", "delivery", "deadLetterSink"]) else {
        return;
    };
    let target = sink
        .get("ref")
        .and_then(|reference| reference_id(reference, trigger_id.namespace.as_deref()))
        .filter(|id| graph.contains_node(id));
    let uri = sink
        .get("uri")
        .and_then(Value::as_str)
        .or_else(|| text_field(trigger, &["status", "deadLetterSinkUri"]));
    if let Some(target) = target {
        let mut edge = new_edge(EdgeKind::DeadLetterSink, &trigger_id, &target);
        if let Some(uri) = uri {
            insert_text_metadata(&mut edge, "uri", uri);
        }
        add_unique_edge(graph, edge, None);
    } else if sink.get("ref").is_some() {
        graph.add_warning(warning(
            "missing_dead_letter_sink",
            format!(
                "Trigger {} dead-letter sink is not in the topology",
                trigger_id.name
            ),
            Some(trigger_id.clone()),
        ));
    }
}

fn add_source_edges(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for source_id in ids {
        let Some(source) = store.get(source_id) else {
            continue;
        };
        let sink = field(&source, &["spec", "sink"]);
        let sink_target = sink
            .and_then(|sink| sink.get("ref"))
            .and_then(|reference| reference_id(reference, source_id.namespace.as_deref()))
            .filter(|id| graph.contains_node(id));
        let sink_uri = sink
            .and_then(|sink| sink.get("uri"))
            .and_then(Value::as_str)
            .or_else(|| text_field(&source, &["status", "sinkUri"]));
        if let Some(target) = sink_target {
            let mut edge = new_edge(EdgeKind::DeliversTo, source_id, &target);
            if let Some(uri) = sink_uri {
                insert_text_metadata(&mut edge, "uri", uri);
            }
            add_unique_edge(graph, edge, None);
        } else if sink.and_then(|sink| sink.get("ref")).is_some() {
            graph.add_warning(warning(
                "missing_sink",
                format!(
                    "Source {} sink reference is not in the topology",
                    source_id.name
                ),
                Some(source_id.clone()),
            ));
        }

        if source_id.kind == "SinkBinding" {
            add_sink_binding_subject(graph, source_id, &source);
        }
    }
}

fn add_sink_binding_subject(
    graph: &mut GraphBuilder,
    binding_id: &ResourceId,
    binding: &DynamicObject,
) {
    let subject_ref = field(binding, &["spec", "subject", "ref"]);
    let subject = subject_ref
        .and_then(|reference| reference_id(reference, binding_id.namespace.as_deref()))
        .filter(|id| graph.contains_node(id));
    if let Some(subject_id) = subject {
        let edge = new_edge(EdgeKind::References, &subject_id, binding_id);
        add_unique_edge(graph, edge, None);
    } else if subject_ref.is_some() {
        graph.add_warning(warning(
            "missing_subject",
            format!(
                "SinkBinding {} subject is not in the topology",
                binding_id.name
            ),
            Some(binding_id.clone()),
        ));
    }
}

fn add_subscription_edges(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for subscription_id in ids {
        let Some(subscription) = store.get(subscription_id) else {
            continue;
        };
        let channel = field(&subscription, &["spec", "channel"])
            .and_then(|reference| reference_id(reference, subscription_id.namespace.as_deref()));
        if let Some(channel_id) = channel.filter(|id| graph.contains_node(id)) {
            let edge = new_edge(EdgeKind::Subscribes, &channel_id, subscription_id);
            add_unique_edge(graph, edge, None);
        } else if field(&subscription, &["spec", "channel"]).is_some() {
            graph.add_warning(warning(
                "missing_channel",
                format!(
                    "Subscription {} channel is not in the topology",
                    subscription_id.name
                ),
                Some(subscription_id.clone()),
            ));
        }

        let subscriber_ref = field(&subscription, &["spec", "subscriber", "ref"]);
        let subscriber_target = subscriber_ref
            .and_then(|reference| reference_id(reference, subscription_id.namespace.as_deref()))
            .filter(|id| graph.contains_node(id));
        let subscriber_uri = subscriber_uri(&subscription);
        let reply = field(&subscription, &["spec", "reply"]);
        if let Some(subscriber_id) = subscriber_target {
            let mut edge = new_edge(EdgeKind::DeliversTo, subscription_id, &subscriber_id);
            if let Some(uri) = subscriber_uri {
                insert_text_metadata(&mut edge, "uri", uri);
            }
            if let Some(reply) = reply {
                insert_metadata(&mut edge, "reply", reply.clone());
            }
            add_unique_edge(graph, edge, None);
        } else if subscriber_ref.is_some() {
            graph.add_warning(warning(
                "missing_subscriber",
                format!(
                    "Subscription {} subscriber reference is not in the topology",
                    subscription_id.name
                ),
                Some(subscription_id.clone()),
            ));
        }

        add_subscription_dead_letter(graph, subscription_id, &subscription);
    }
}

fn add_subscription_dead_letter(
    graph: &mut GraphBuilder,
    subscription_id: &ResourceId,
    subscription: &DynamicObject,
) {
    let Some(sink) = field(subscription, &["spec", "delivery", "deadLetterSink"]) else {
        return;
    };
    let target = sink
        .get("ref")
        .and_then(|reference| reference_id(reference, subscription_id.namespace.as_deref()))
        .filter(|id| graph.contains_node(id));
    let uri = sink
        .get("uri")
        .and_then(Value::as_str)
        .or_else(|| text_field(subscription, &["status", "deadLetterSinkUri"]));
    if let Some(target) = target {
        let mut edge = new_edge(EdgeKind::DeadLetterSink, subscription_id, &target);
        if let Some(uri) = uri {
            insert_text_metadata(&mut edge, "uri", uri);
        }
        add_unique_edge(graph, edge, None);
    } else if sink.get("ref").is_some() {
        graph.add_warning(warning(
            "missing_dead_letter_sink",
            format!(
                "Subscription {} dead-letter sink is not in the topology",
                subscription_id.name
            ),
            Some(subscription_id.clone()),
        ));
    }
}

fn add_channel_backing_edges(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for channel_id in ids {
        let Some(channel) = store.get(channel_id) else {
            continue;
        };
        let backing = field(&channel, &["status", "channel"])
            .and_then(|reference| reference_id(reference, channel_id.namespace.as_deref()))
            .filter(|id| graph.contains_node(id));
        let Some(backing_id) = backing else {
            continue;
        };
        let mut edge = new_edge(EdgeKind::BackedBy, channel_id, &backing_id);
        if let Some(address) = text_field(&channel, &["status", "address", "url"]) {
            insert_text_metadata(&mut edge, "address", address);
        }
        add_unique_edge(graph, edge, None);
    }
}

fn trigger_broker_id(
    graph: &GraphBuilder,
    trigger: &DynamicObject,
    trigger_id: &ResourceId,
) -> Option<ResourceId> {
    if let Some(name) = text_field(trigger, &["spec", "broker"]) {
        let candidate = namespaced(
            EVENTING_API,
            "Broker",
            trigger_id.namespace.as_deref(),
            name,
        );
        if graph.contains_node(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(reference) = field(trigger, &["spec", "brokerRef"])
        && let Some(candidate) = broker_reference_id(reference, trigger_id.namespace.as_deref())
        && graph.contains_node(&candidate)
    {
        return Some(candidate);
    }
    None
}

fn broker_reference_id(reference: &Value, default_namespace: Option<&str>) -> Option<ResourceId> {
    let name = reference.get("name").and_then(Value::as_str)?;
    let namespace = reference
        .get("namespace")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| default_namespace.map(ToOwned::to_owned));
    let api_version = reference
        .get("apiVersion")
        .and_then(Value::as_str)
        .unwrap_or(EVENTING_API);
    let kind = reference
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("Broker");
    Some(ResourceId::new(api_version, kind, namespace, name))
}

fn trigger_filters(object: &DynamicObject) -> (Option<Value>, usize) {
    let attributes = field(object, &["spec", "filter", "attributes"]).cloned();
    let filters = field(object, &["spec", "filters"]).cloned();
    let attributes_count = attributes
        .as_ref()
        .and_then(Value::as_object)
        .map_or(0, serde_json::Map::len);
    let filters_count = filters
        .as_ref()
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let value = match (attributes, filters) {
        (Some(attributes), Some(filters)) => {
            Some(json!({ "attributes": attributes, "filters": filters }))
        }
        (Some(attributes), None) => Some(attributes),
        (None, Some(filters)) => Some(filters),
        (None, None) => None,
    };
    (value, attributes_count + filters_count)
}

fn subscriber_summary(object: &DynamicObject) -> String {
    if let Some(uri) = subscriber_uri(object) {
        return uri.to_owned();
    }
    field(object, &["spec", "subscriber", "ref"])
        .and_then(Value::as_object)
        .map(|reference| {
            let kind = reference.get("kind").and_then(Value::as_str).unwrap_or("?");
            let name = reference.get("name").and_then(Value::as_str).unwrap_or("?");
            format!("{kind}/{name}")
        })
        .unwrap_or_else(|| "no subscriber".to_owned())
}

fn subscriber_uri(object: &DynamicObject) -> Option<&str> {
    text_field(object, &["spec", "subscriber", "uri"])
        .or_else(|| text_field(object, &["status", "subscriberUri"]))
}

fn sink_summary(object: &DynamicObject) -> String {
    text_field(object, &["status", "sinkUri"])
        .or_else(|| text_field(object, &["spec", "sink", "uri"]))
        .map(ToOwned::to_owned)
        .or_else(|| {
            field(object, &["spec", "sink", "ref"])
                .and_then(Value::as_object)
                .map(|reference| {
                    let kind = reference.get("kind").and_then(Value::as_str).unwrap_or("?");
                    let name = reference.get("name").and_then(Value::as_str).unwrap_or("?");
                    format!("{kind}/{name}")
                })
        })
        .unwrap_or_else(|| "no sink".to_owned())
}

fn broker_class(object: &DynamicObject) -> &str {
    object
        .annotations()
        .get(BROKER_CLASS_ANNOTATION)
        .map_or(DEFAULT_BROKER_CLASS, String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::{ObjectMeta, TypeMeta};

    fn object(data: Value) -> DynamicObject {
        DynamicObject {
            types: Some(TypeMeta {
                api_version: "eventing.knative.dev/v1".to_owned(),
                kind: "Trigger".to_owned(),
            }),
            metadata: ObjectMeta {
                name: Some("trigger".to_owned()),
                namespace: Some("default".to_owned()),
                ..ObjectMeta::default()
            },
            data,
        }
    }

    #[test]
    fn filter_shapes_merge_attributes_and_lists() {
        let attributes = object(json!({
            "spec": { "filter": { "attributes": { "type": "a", "source": "b" } } }
        }));
        let (value, count) = trigger_filters(&attributes);
        assert_eq!(count, 2);
        assert_eq!(
            value.and_then(|value| value.get("type").cloned()),
            Some(json!("a"))
        );

        let filters = object(json!({
            "spec": { "filters": [{ "exact": { "type": "a" } }] }
        }));
        let (value, count) = trigger_filters(&filters);
        assert_eq!(count, 1);
        assert!(value.is_some_and(|value| value.is_array()));

        let combined = object(json!({
            "spec": {
                "filter": { "attributes": { "type": "a" } },
                "filters": [{ "exact": { "source": "b" } }]
            }
        }));
        let (value, count) = trigger_filters(&combined);
        assert_eq!(count, 2);
        assert_eq!(
            value.and_then(|value| value.get("attributes").cloned()),
            Some(json!({ "type": "a" }))
        );
    }

    #[test]
    fn broker_references_default_to_the_eventing_group() {
        let id = broker_reference_id(&json!({ "name": "mt-broker" }), Some("default"))
            .expect("broker reference resolves");
        assert_eq!(id.api_version, EVENTING_API);
        assert_eq!(id.kind, "Broker");
        assert_eq!(id.namespace.as_deref(), Some("default"));
        assert_eq!(id.name, "mt-broker");
    }

    #[test]
    fn broker_class_defaults_to_mt_channel() {
        let mut broker = object(json!({}));
        assert_eq!(broker_class(&broker), DEFAULT_BROKER_CLASS);
        broker.metadata.annotations = Some(
            [(BROKER_CLASS_ANNOTATION.to_owned(), "Kafka".to_owned())]
                .into_iter()
                .collect(),
        );
        assert_eq!(broker_class(&broker), "Kafka");
    }
}
