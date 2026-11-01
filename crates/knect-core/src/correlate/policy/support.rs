//! Shared helpers for the Istio, Flagger, and Kyverno policy correlators.
//!
//! The helpers stay pure: they only read the sanitized resource store and
//! return canonical identities or redacted strings.

use std::collections::{BTreeMap, BTreeSet};

use knect_proto::ResourceId;
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::Value;
use url::Url;

use crate::graph::support::{field, text_field};
use crate::store::{ResourceStore, matches_labels};

/// Istio networking API group.
pub const ISTIO_NETWORKING_GROUP: &str = "networking.istio.io";
/// Istio security API group.
pub const ISTIO_SECURITY_GROUP: &str = "security.istio.io";
/// Flagger API group.
pub const FLAGGER_GROUP: &str = "flagger.app";
/// Kyverno API group.
pub const KYVERNO_GROUP: &str = "kyverno.io";
/// Policy report API group.
pub const POLICY_REPORT_GROUP: &str = "wgpolicyk8s.io";
/// Knative Serving API group.
pub const SERVING_GROUP: &str = "serving.knative.dev";
/// Knative Serving API version.
pub const SERVING_API: &str = "serving.knative.dev/v1";

/// Returns every identity in a group/kind pair across API versions.
#[must_use]
pub fn ids_by_group_kind(store: &ResourceStore, group: &str, kind: &str) -> Vec<ResourceId> {
    store
        .ids()
        .filter(|id| id.kind == kind && id.gvk().group == group)
        .cloned()
        .collect()
}

/// Returns every identity for a kind regardless of API group.
#[must_use]
pub fn ids_by_kind(store: &ResourceStore, kind: &str) -> Vec<ResourceId> {
    store.ids().filter(|id| id.kind == kind).cloned().collect()
}

/// Extracts a lowercase host from a URL or a bare host string.
#[must_use]
pub fn host_from_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(trimmed)
        && let Some(host) = parsed.host_str()
    {
        return Some(normalize_host(host));
    }
    let without_path = trimmed.split('/').next()?;
    let without_port = without_path
        .rsplit_once(':')
        .map_or(without_path, |(host, _)| host);
    (!without_port.is_empty()).then(|| normalize_host(without_port))
}

fn normalize_host(value: &str) -> String {
    value.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Index of Knative Service and Route hosts, keyed by DNS name.
pub type HostIndex = BTreeMap<String, BTreeSet<ResourceId>>;

/// Builds the host index from `status.url` and route traffic URLs.
#[must_use]
pub fn knative_host_index(store: &ResourceStore) -> HostIndex {
    let mut index = HostIndex::new();
    for id in store.ids() {
        if id.gvk().group != SERVING_GROUP || (id.kind != "Service" && id.kind != "Route") {
            continue;
        }
        let Some(object) = store.get(id) else {
            continue;
        };
        insert_status_host(&mut index, &object, id);
    }
    index
}

fn insert_status_host(index: &mut HostIndex, object: &DynamicObject, id: &ResourceId) {
    for path in [&["status", "url"][..], &["status", "address", "url"][..]] {
        if let Some(host) = text_field(object, path).and_then(host_from_url) {
            index.entry(host).or_default().insert(id.clone());
        }
    }
    let Some(traffic) = field(object, &["status", "traffic"]).and_then(Value::as_array) else {
        return;
    };
    for entry in traffic {
        if let Some(host) = entry
            .get("url")
            .and_then(Value::as_str)
            .and_then(host_from_url)
        {
            index.entry(host).or_default().insert(id.clone());
        }
    }
}

/// Resolves Istio destination or host strings to known Knative identities.
pub struct HostResolver<'a> {
    store: &'a ResourceStore,
    hosts: &'a HostIndex,
}

impl<'a> HostResolver<'a> {
    /// Creates a resolver over a store and its Knative host index.
    #[must_use]
    pub fn new(store: &'a ResourceStore, hosts: &'a HostIndex) -> Self {
        Self { store, hosts }
    }

    /// Resolves one host to every matching Service, Route, Configuration, or
    /// Revision identity. `namespace` is the fallback for short service names.
    #[must_use]
    pub fn resolve(&self, host: &str, namespace: Option<&str>) -> Vec<ResourceId> {
        let Some(host) = host_from_url(host) else {
            return Vec::new();
        };
        if host.is_empty() || matches!(host.as_str(), "mesh" | "passthrough") {
            return Vec::new();
        }

        let mut found = BTreeSet::new();
        if let Some(ids) = self.hosts.get(&host) {
            found.extend(ids.iter().cloned());
        }

        let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
        let service_host = host.contains(".svc.") || host.ends_with(".svc") || labels.len() == 1;
        if !service_host {
            return found.into_iter().collect();
        }
        let Some(name) = labels.first() else {
            return found.into_iter().collect();
        };
        let namespace = labels.get(1).copied().or(namespace).map(ToOwned::to_owned);
        for kind in ["Revision", "Service", "Route", "Configuration"] {
            let candidate = ResourceId::new(SERVING_API, kind, namespace.clone(), *name);
            if self.store.contains(&candidate) {
                found.insert(candidate);
            }
        }
        found.into_iter().collect()
    }
}

/// Reads `spec.selector.matchLabels` into a deterministic map.
#[must_use]
pub fn selector_labels(object: &DynamicObject) -> BTreeMap<String, String> {
    field(object, &["spec", "selector", "matchLabels"])
        .and_then(Value::as_object)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns true when the object carries every selector label.
#[must_use]
pub fn object_matches(object: &DynamicObject, selector: &BTreeMap<String, String>) -> bool {
    matches_labels(object, selector)
}

/// Returns the full condition array as JSON, preserving its order.
#[must_use]
pub fn conditions_json(object: &DynamicObject) -> Value {
    field(object, &["status", "conditions"])
        .and_then(Value::as_array)
        .map(|conditions| Value::Array(conditions.clone()))
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

/// Extracts the `hosts` array from an Istio VirtualService.
#[must_use]
pub fn vs_hosts(object: &DynamicObject) -> Vec<String> {
    field(object, &["spec", "hosts"])
        .and_then(Value::as_array)
        .map(|hosts| {
            hosts
                .iter()
                .filter_map(Value::as_str)
                .filter_map(host_from_url)
                .collect()
        })
        .unwrap_or_default()
}

/// True when the object is a Knative net-istio or Flagger-generated resource.
#[must_use]
pub fn managed_origin(object: &DynamicObject) -> &'static str {
    if object
        .owner_references()
        .iter()
        .any(|owner| owner.kind == "Canary")
    {
        return "flagger";
    }
    if object.labels().keys().any(|key| {
        key.starts_with("networking.internal.knative.dev/") || key == "serving.knative.dev/route"
    }) {
        return "knative";
    }
    "custom"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_normalized_from_urls_and_bare_names() {
        assert_eq!(
            host_from_url("http://order.default.svc.cluster.local:80/path"),
            Some("order.default.svc.cluster.local".to_owned())
        );
        assert_eq!(
            host_from_url("order.default.svc.cluster.local:80"),
            Some("order.default.svc.cluster.local".to_owned())
        );
        assert_eq!(host_from_url("Mesh."), Some("mesh".to_owned()));
        assert_eq!(host_from_url("   "), None);
    }

    #[test]
    fn host_resolver_maps_service_and_revision_hosts() {
        use kube::api::{ObjectMeta, TypeMeta};

        let mut store = ResourceStore::new();
        let revision = DynamicObject {
            types: Some(TypeMeta {
                api_version: SERVING_API.to_owned(),
                kind: "Revision".to_owned(),
            }),
            metadata: ObjectMeta {
                name: Some("order-00001".to_owned()),
                namespace: Some("default".to_owned()),
                ..ObjectMeta::default()
            },
            data: serde_json::json!({}),
        };
        store.upsert(revision);
        let index = knative_host_index(&store);
        let resolver = HostResolver::new(&store, &index);
        assert_eq!(
            resolver.resolve("order-00001.default.svc.cluster.local", None),
            vec![ResourceId::namespaced(
                SERVING_API,
                "Revision",
                "default",
                "order-00001"
            )]
        );
        assert!(resolver.resolve("mesh", None).is_empty());
        assert!(resolver.resolve("orders.example.com", None).is_empty());
    }
}
