//! Resolution indexes for operator CRDs, Services, and broker endpoints.
//!
//! Operator CRDs create predictable Services (`<cluster>-kafka-bootstrap`,
//! `<cluster>-rw`, or the cluster name itself). The index matches those names
//! without inventing identities: every resolved target is an existing operator
//! node, and a Service that does not match a node resolves to nothing.

use std::collections::{BTreeMap, BTreeSet};

use knect_proto::ResourceId;
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::Value;
use url::Url;

use crate::graph::support::{field, text_field};
use crate::operators::{OperatorFamily, OperatorNode};
use crate::store::ResourceStore;

/// Deterministic index of cluster-family operator nodes.
pub(crate) struct ClusterIndex<'a> {
    nodes: Vec<&'a OperatorNode>,
}

impl<'a> ClusterIndex<'a> {
    /// Builds an index sorted by longest name first for stable prefix matches.
    pub(crate) fn new(nodes: &'a [OperatorNode]) -> Self {
        let mut clusters: Vec<&OperatorNode> = nodes
            .iter()
            .filter(|node| node.family.is_cluster())
            .collect();
        clusters.sort_by(|left, right| {
            right
                .id
                .name
                .len()
                .cmp(&left.id.name.len())
                .then_with(|| left.id.cmp(&right.id))
        });
        Self { nodes: clusters }
    }

    /// Finds a cluster by exact namespace and name.
    pub(crate) fn by_name(&self, namespace: Option<&str>, name: &str) -> Option<&'a OperatorNode> {
        self.nodes
            .iter()
            .copied()
            .find(|node| node.id.namespace.as_deref() == namespace && node.id.name == name)
    }

    fn matching(&self, service: &str) -> Option<&'a OperatorNode> {
        self.nodes
            .iter()
            .copied()
            .filter_map(|node| match_score(service, &node.id.name).map(|score| (score, node)))
            .min_by(|(left_score, left), (right_score, right)| {
                left_score
                    .cmp(right_score)
                    .then_with(|| right.id.name.len().cmp(&left.id.name.len()))
                    .then_with(|| left.id.cmp(&right.id))
            })
            .map(|(_score, node)| node)
    }
}

/// Deterministic index of topic-family operator nodes.
pub(crate) struct TopicIndex<'a> {
    nodes: Vec<&'a OperatorNode>,
}

impl<'a> TopicIndex<'a> {
    /// Builds the topic index in identity order.
    pub(crate) fn new(nodes: &'a [OperatorNode]) -> Self {
        let nodes: Vec<&OperatorNode> =
            nodes.iter().filter(|node| node.family.is_topic()).collect();
        Self { nodes }
    }

    /// Finds a topic by exact namespace and name.
    pub(crate) fn by_name(&self, namespace: Option<&str>, name: &str) -> Option<&'a OperatorNode> {
        self.nodes
            .iter()
            .copied()
            .find(|node| node.id.namespace.as_deref() == namespace && node.id.name == name)
    }

    /// Iterates over the indexed topics.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &&'a OperatorNode> {
        self.nodes.iter()
    }
}

/// Join of Service/Endpoints names and cluster-family nodes.
pub(crate) struct BackingIndex<'a> {
    /// Cluster index retained for exact-name topic lookups.
    pub(crate) clusters: ClusterIndex<'a>,
    by_service: BTreeMap<(Option<String>, String), &'a OperatorNode>,
}

impl<'a> BackingIndex<'a> {
    /// Correlates every core Service and Endpoints name with the best cluster.
    pub(crate) fn new(clusters: ClusterIndex<'a>, services: &[ResourceId]) -> Self {
        let mut by_service = BTreeMap::new();
        for service in services {
            if let Some(cluster) = clusters.matching(&service.name) {
                by_service.insert((service.namespace.clone(), service.name.clone()), cluster);
            }
        }
        Self {
            clusters,
            by_service,
        }
    }

    /// Finds the cluster backing a Service or Endpoints name.
    pub(crate) fn for_service(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<&'a OperatorNode> {
        self.by_service
            .get(&(namespace.map(ToOwned::to_owned), name.to_owned()))
            .copied()
    }
}

/// Core `Service` and `Endpoints` identities in the store.
pub(crate) fn service_ids(store: &ResourceStore) -> Vec<ResourceId> {
    let mut ids = BTreeSet::new();
    for (id, _) in store.iter() {
        if id.api_version == "v1" && matches!(id.kind.as_str(), "Service" | "Endpoints") {
            ids.insert(id.clone());
        }
    }
    ids.into_iter().collect()
}

/// Extracts candidate `(namespace, service)` pairs from a Broker.
///
/// The first source is the Broker address URL; the second is the Kafka
/// `bootstrap.servers` annotation. Both are normalised to a Service name so the
/// same backing index resolves Strimzi, Redpanda, and RabbitMQ clusters.
pub(crate) fn broker_service_hosts(
    object: &DynamicObject,
    namespace: Option<&str>,
) -> Vec<(Option<String>, String)> {
    let mut hosts = Vec::new();
    if let Some(url) = text_field(object, &["status", "address", "url"])
        && let Some(host) = service_from_host(url, namespace)
    {
        hosts.push(host);
    }
    if let Some(servers) = text_field(object, &["status", "annotations", "bootstrap.servers"]) {
        for server in servers.split(',') {
            if let Some(host) = service_from_host(server.trim(), namespace) {
                hosts.push(host);
            }
        }
    }
    hosts
}

/// Resolves the cluster name referenced by a topic CRD.
pub(crate) fn topic_cluster_name(object: &DynamicObject) -> Option<String> {
    const LABEL_KEYS: [&str; 4] = [
        "strimzi.io/cluster",
        "kafka.strimzi.io/cluster",
        "cluster.redpanda.com/cluster",
        "redpanda.vectorized.io/cluster",
    ];
    let labels = object.labels();
    for key in LABEL_KEYS {
        if let Some(value) = labels.get(key) {
            return Some(value.clone());
        }
    }
    const PATHS: [&[&str]; 3] = [
        &["spec", "clusterName"],
        &["spec", "clusterRef", "name"],
        &["spec", "cluster", "name"],
    ];
    PATHS.iter().find_map(|path| {
        field(object, path)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

/// True when a cluster family can back a Broker of the given class.
pub(crate) fn broker_cluster_compatible(class_lower: &str, family: OperatorFamily) -> bool {
    if class_lower.contains("rabbitmq") {
        family == OperatorFamily::RabbitmqCluster
    } else if class_lower.contains("kafka") {
        matches!(
            family,
            OperatorFamily::StrimziKafka | OperatorFamily::RedpandaCluster
        )
    } else {
        false
    }
}

/// True when a topic family is backed by the cluster family.
pub(crate) fn topic_cluster_compatible(topic: OperatorFamily, cluster: OperatorFamily) -> bool {
    matches!(
        (topic, cluster),
        (OperatorFamily::StrimziTopic, OperatorFamily::StrimziKafka)
            | (
                OperatorFamily::RedpandaTopic,
                OperatorFamily::RedpandaCluster
            )
    )
}

/// Returns the API group of an `apiVersion` string.
pub(crate) fn api_group(api_version: &str) -> &str {
    api_version
        .split_once('/')
        .map_or("", |(group, _version)| group)
}

/// True when the identity is a Knative Eventing Broker.
pub(crate) fn is_broker_id(id: &ResourceId) -> bool {
    api_group(&id.api_version) == "eventing.knative.dev" && id.kind == "Broker"
}

/// Broker class annotation with the Knative default.
pub(crate) fn broker_class(object: &DynamicObject) -> &str {
    object
        .annotations()
        .get("eventing.knative.dev/broker.class")
        .map_or("MTChannelBasedBroker", String::as_str)
}

fn service_from_host(
    value: &str,
    default_namespace: Option<&str>,
) -> Option<(Option<String>, String)> {
    let host = host_of(value)?;
    let host = host.trim_end_matches('.');
    let mut parts = host.split('.');
    let service = parts.next()?.to_owned();
    if service.is_empty() {
        return None;
    }
    let namespace = parts
        .next()
        .filter(|part| !part.is_empty() && *part != "svc")
        .map(ToOwned::to_owned)
        .or_else(|| default_namespace.map(ToOwned::to_owned));
    Some((namespace, service))
}

fn host_of(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains("://")
        && let Ok(url) = Url::parse(value)
    {
        return url.host_str().map(ToOwned::to_owned);
    }
    let host = match value.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.chars().all(|character| character.is_ascii_digit()) =>
        {
            host
        }
        _ => value,
    };
    Some(host.trim_end_matches('.').to_owned())
}

/// Scores how well a Service name matches a cluster name.
///
/// Exact names win, followed by `<cluster>-*` (operator-generated Services), and
/// finally `<service>-*` (a shorter Service label naming a compound cluster).
fn match_score(service: &str, cluster: &str) -> Option<u8> {
    if service == cluster {
        Some(0)
    } else if service
        .strip_prefix(cluster)
        .is_some_and(|rest| rest.starts_with('-'))
    {
        Some(1)
    } else if cluster
        .strip_prefix(service)
        .is_some_and(|rest| rest.starts_with('-'))
    {
        Some(2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing_handles_urls_ports_and_fqdns() {
        assert_eq!(
            service_from_host(
                "http://orders-kafka-kafka-bootstrap.knect-e2e.svc.cluster.local:9092",
                None
            ),
            Some((
                Some("knect-e2e".to_owned()),
                "orders-kafka-kafka-bootstrap".to_owned()
            ))
        );
        assert_eq!(
            service_from_host("redpanda.knect-e2e.svc.cluster.local:9092", Some("default")),
            Some((Some("knect-e2e".to_owned()), "redpanda".to_owned()))
        );
        assert_eq!(
            service_from_host("redis:6379", Some("default")),
            Some((Some("default".to_owned()), "redis".to_owned()))
        );
        assert_eq!(service_from_host("", None), None);
    }

    #[test]
    fn service_matching_prefers_exact_names() {
        assert_eq!(match_score("redis", "redis"), Some(0));
        assert_eq!(
            match_score("orders-kafka-kafka-bootstrap", "orders-kafka"),
            Some(1)
        );
        assert_eq!(match_score("redis", "redis-cluster"), Some(2));
        assert_eq!(match_score("orders", "payments"), None);
    }
}
