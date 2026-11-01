//! Store indexes used to resolve connection targets.
//!
//! Only objects that can semantically terminate a connection are indexed:
//! core `Service`/`Endpoints`, and backing-operator custom resources. Arbitrary
//! workload names never become link targets.

use std::collections::{BTreeMap, BTreeSet};

use knect_proto::ResourceId;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::graph::support::field;
use crate::store::ResourceStore;

/// Outcome of resolving a scanned name or address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// An existing object in the sanitized store.
    Resolved(ResourceId),
    /// A strong reference to a name that should have existed.
    Missing,
    /// An external name outside the observed namespaces.
    External,
}

/// Deterministic index of connection-capable objects.
#[derive(Debug, Clone, Default)]
pub(crate) struct TargetIndex {
    namespaces: BTreeSet<String>,
    services: BTreeMap<(String, String), ResourceId>,
    endpoints: BTreeMap<(String, String), ResourceId>,
    addresses: BTreeMap<(String, String), ResourceId>,
    backing: BTreeMap<(String, String), ResourceId>,
}

impl TargetIndex {
    /// Builds the index from a sanitized store snapshot.
    pub(crate) fn build(store: &ResourceStore) -> Self {
        let mut index = Self::default();
        for (id, _) in store.iter() {
            if let Some(namespace) = &id.namespace {
                index.namespaces.insert(namespace.clone());
            }
        }
        for (id, _) in store.iter() {
            let Some(namespace) = id.namespace.clone() else {
                continue;
            };
            let key = (namespace, id.name.clone());
            if is_core_service(id) {
                index.services.insert(key, id.clone());
            } else if is_endpoints(id) {
                index.endpoints.insert(key, id.clone());
            } else if is_backing_object(id) {
                index.backing.insert(key, id.clone());
            }
        }
        for (id, object) in store.iter() {
            let Some(namespace) = id.namespace.clone() else {
                continue;
            };
            if !is_endpoints(id) {
                continue;
            }
            let service_key = (namespace.clone(), id.name.clone());
            let target = index
                .services
                .get(&service_key)
                .cloned()
                .unwrap_or_else(|| id.clone());
            for address in endpoint_addresses(object) {
                index
                    .addresses
                    .insert((namespace.clone(), address), target.clone());
            }
        }
        index
    }

    /// Returns true when a namespace has been observed in the store.
    pub(crate) fn has_namespace(&self, namespace: &str) -> bool {
        self.namespaces.contains(namespace)
    }

    /// Resolves a DNS-style service name.
    pub(crate) fn resolve_service(
        &self,
        name: &str,
        namespace: Option<&str>,
        source_namespace: Option<&str>,
    ) -> Lookup {
        if let Some(namespace) = namespace
            && !self.has_namespace(namespace)
        {
            return Lookup::External;
        }
        let namespace = namespace
            .map(ToOwned::to_owned)
            .or_else(|| source_namespace.map(ToOwned::to_owned));
        let Some(namespace) = namespace else {
            return Lookup::Missing;
        };
        let key = (namespace, name.to_ascii_lowercase());
        if let Some(id) = self.services.get(&key) {
            return Lookup::Resolved(id.clone());
        }
        if let Some(id) = self.endpoints.get(&key) {
            return Lookup::Resolved(id.clone());
        }
        if let Some(id) = self.backing.get(&key) {
            return Lookup::Resolved(id.clone());
        }
        Lookup::Missing
    }

    /// Resolves a literal IP address against observed `Endpoints` addresses.
    pub(crate) fn resolve_address(&self, address: &str, source_namespace: Option<&str>) -> Lookup {
        if let Some(namespace) = source_namespace
            && let Some(id) = self
                .addresses
                .get(&(namespace.to_owned(), address.to_owned()))
        {
            return Lookup::Resolved(id.clone());
        }
        let matches: BTreeSet<&ResourceId> = self
            .addresses
            .iter()
            .filter(|((_namespace, candidate), _id)| candidate == address)
            .map(|(_key, id)| id)
            .collect();
        match matches.len() {
            0 => Lookup::Missing,
            1 => matches
                .into_iter()
                .next()
                .map_or(Lookup::Missing, |id| Lookup::Resolved(id.clone())),
            _ => Lookup::External,
        }
    }
}

/// Returns the API group for an `apiVersion` string.
pub(crate) fn api_group(api_version: &str) -> &str {
    api_version
        .split_once('/')
        .map_or("", |(group, _version)| group)
}

/// Identities of every workload that can carry a Pod template.
pub(crate) fn workload_ids(store: &ResourceStore) -> Vec<ResourceId> {
    store
        .iter()
        .filter(|(id, _)| is_workload_source(id))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Returns the Pod spec carried by a workload object.
pub(crate) fn pod_spec<'a>(id: &ResourceId, object: &'a DynamicObject) -> Option<&'a Value> {
    match (api_group(&id.api_version), id.kind.as_str()) {
        ("serving.knative.dev", "Revision") | ("", "Pod") => field(object, &["spec"]),
        ("batch", "CronJob") => field(object, &["spec", "jobTemplate", "spec", "template", "spec"]),
        ("batch", "Job") => field(object, &["spec", "template", "spec"]),
        _ => field(object, &["spec", "template", "spec"]),
    }
}

/// Returns the JSON Pointer prefix of a workload's Pod spec.
pub(crate) fn pod_spec_prefix(id: &ResourceId) -> &'static str {
    match (api_group(&id.api_version), id.kind.as_str()) {
        ("serving.knative.dev", "Revision") | ("", "Pod") => "/spec",
        ("batch", "CronJob") => "/spec/jobTemplate/spec/template/spec",
        _ => "/spec/template/spec",
    }
}

fn is_workload_source(id: &ResourceId) -> bool {
    matches!(
        (api_group(&id.api_version), id.kind.as_str()),
        ("serving.knative.dev", "Revision")
            | ("", "Pod")
            | (
                "apps",
                "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet"
            )
            | ("batch", "Job" | "CronJob")
    )
}

fn is_core_service(id: &ResourceId) -> bool {
    id.api_version == "v1" && id.kind == "Service"
}

fn is_endpoints(id: &ResourceId) -> bool {
    id.api_version == "v1" && id.kind == "Endpoints"
}

/// Recognizes backing-service custom resources without matching workloads.
fn is_backing_object(id: &ResourceId) -> bool {
    let group = api_group(&id.api_version);
    if group.is_empty() {
        return false;
    }
    const KINDS: &[&str] = &[
        "Redis",
        "RedisCluster",
        "RedisFailover",
        "Dragonfly",
        "Kafka",
        "KafkaTopic",
        "Topic",
        "RabbitmqCluster",
        "CephBlockPool",
        "CephFilesystem",
        "CephObjectStore",
    ];
    const GROUP_HINTS: &[&str] = &[
        "redis", "kafka", "strimzi", "rabbitmq", "cnpg", "postgres", "redpanda", "rook", "ceph",
    ];
    KINDS.contains(&id.kind.as_str()) || GROUP_HINTS.iter().any(|hint| group.contains(hint))
}

fn endpoint_addresses(object: &DynamicObject) -> Vec<String> {
    let Some(subsets) = field(object, &["subsets"]).and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut addresses = Vec::new();
    for subset in subsets {
        let Some(entries) = subset.get("addresses").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            for key in ["ip", "hostname"] {
                if let Some(value) = entry.get(key).and_then(Value::as_str) {
                    addresses.push(value.to_ascii_lowercase());
                }
            }
        }
    }
    addresses.sort();
    addresses.dedup();
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource::tests::test_object;

    fn service(namespace: &str, name: &str) -> DynamicObject {
        test_object("v1", "Service", Some(namespace), name)
    }

    #[test]
    fn resolves_services_endpoints_and_backing_objects() {
        let mut store = ResourceStore::new();
        store.upsert(service("knect-e2e", "redis-master"));
        store.upsert(test_object(
            "redis.redis.opstreelabs.in/v1beta2",
            "RedisCluster",
            Some("knect-e2e"),
            "redis-cache",
        ));
        let index = TargetIndex::build(&store);

        assert_eq!(
            index.resolve_service("redis-master", None, Some("knect-e2e")),
            Lookup::Resolved(ResourceId::namespaced(
                "v1",
                "Service",
                "knect-e2e",
                "redis-master"
            ))
        );
        assert!(matches!(
            index.resolve_service("redis-cache", None, Some("knect-e2e")),
            Lookup::Resolved(_)
        ));
        assert_eq!(
            index.resolve_service("redis-master", Some("knect-e2e"), None),
            Lookup::Resolved(ResourceId::namespaced(
                "v1",
                "Service",
                "knect-e2e",
                "redis-master"
            ))
        );
        assert_eq!(
            index.resolve_service("unknown", None, Some("knect-e2e")),
            Lookup::Missing
        );
        assert_eq!(
            index.resolve_service("unknown", Some("elsewhere"), None),
            Lookup::External
        );
    }

    #[test]
    fn workload_sources_exclude_plain_services() {
        let mut store = ResourceStore::new();
        store.upsert(service("default", "redis-master"));
        store.upsert(test_object(
            "serving.knative.dev/v1",
            "Revision",
            Some("default"),
            "hello-00001",
        ));
        store.upsert(test_object("v1", "Pod", Some("default"), "hello-pod"));
        let ids = workload_ids(&store);
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| id.kind != "Service"));
    }

    #[test]
    fn endpoint_addresses_include_ips_and_hostnames() {
        let mut endpoints = test_object("v1", "Endpoints", Some("default"), "redis-master");
        endpoints.data = serde_json::json!({
            "subsets": [{
                "addresses": [
                    {"ip": "10.0.0.10", "hostname": "redis-primary"},
                    {"ip": "10.0.0.11"}
                ]
            }]
        });
        assert_eq!(
            endpoint_addresses(&endpoints),
            vec![
                "10.0.0.10".to_owned(),
                "10.0.0.11".to_owned(),
                "redis-primary".to_owned()
            ]
        );
    }
}
