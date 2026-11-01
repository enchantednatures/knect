//! Istio traffic edges: VirtualService and DestinationRule targets.

use std::collections::BTreeSet;

use knect_proto::ResourceId;
use knect_proto::graph::EdgeKind;
use kube::ResourceExt;
use serde_json::{Value, json};

use crate::graph::GraphBuilder;
use crate::graph::support::{
    add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge, text_field,
};
use crate::store::ResourceStore;

use super::super::support::{
    HostResolver, SERVING_API, SERVING_GROUP, host_from_url, ids_by_group_kind, managed_origin,
};
use super::TrafficInputs;

pub(super) fn add_traffic_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    inputs: TrafficInputs<'_>,
) {
    let resolver = HostResolver::new(store, inputs.hosts);
    for id in inputs.virtual_services {
        let Some(object) = store.get(id) else {
            continue;
        };
        add_virtual_service_edges(store, graph, id, &object, &resolver, inputs.conflicts);
    }
    for id in inputs.destination_rules {
        let Some(object) = store.get(id) else {
            continue;
        };
        add_destination_rule_edges(graph, id, &object, &resolver);
    }
}

fn add_virtual_service_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    id: &ResourceId,
    object: &kube::api::DynamicObject,
    resolver: &HostResolver<'_>,
    conflicts: &super::Conflicts,
) {
    let targets = vs_targets(store, id, object, resolver);
    let hosts = super::super::support::vs_hosts(object);
    let origin = managed_origin(object);
    let http_routes = field(object, &["spec", "http"])
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let targets: Vec<ResourceId> = targets
        .iter()
        .filter(|target| graph.contains_node(target))
        .cloned()
        .collect();
    for target in &targets {
        let mut edge = new_edge(EdgeKind::Manages, id, target);
        insert_metadata(&mut edge, "hosts", json!(hosts));
        insert_text_metadata(&mut edge, "managed_by", origin);
        insert_metadata(&mut edge, "http_routes", json!(http_routes));
        insert_metadata(&mut edge, "host_conflict", json!(false));
        if let Some(conflict) = conflicts.get(id).and_then(|entries| {
            entries
                .iter()
                .find(|conflict| conflict.targets.contains(target))
        }) {
            insert_metadata(&mut edge, "host_conflict", json!(true));
            insert_text_metadata(&mut edge, "conflict_host", &conflict.host);
            insert_text_metadata(&mut edge, "conflict_with", &target.to_string());
        }
        add_unique_edge(graph, edge, None);
    }
    add_destination_targets(graph, id, object, resolver, &hosts);
}

fn vs_targets(
    store: &ResourceStore,
    id: &ResourceId,
    object: &kube::api::DynamicObject,
    resolver: &HostResolver<'_>,
) -> BTreeSet<ResourceId> {
    let mut targets = BTreeSet::new();
    let labels = object.labels();
    if let Some(name) = labels.get("serving.knative.dev/route") {
        let namespace = labels
            .get("serving.knative.dev/routeNamespace")
            .cloned()
            .or_else(|| id.namespace.clone());
        targets.extend(route_target(store, namespace, name));
    }
    for owner in object.owner_references() {
        if owner.kind == "Ingress" {
            targets.extend(route_target(store, id.namespace.clone(), &owner.name));
        }
    }
    let namespace = id.namespace.as_deref();
    for host in super::super::support::vs_hosts(object) {
        targets.extend(
            resolver
                .resolve(&host, namespace)
                .into_iter()
                .filter(is_knative_target),
        );
    }
    for service in ids_by_group_kind(store, SERVING_GROUP, "Service") {
        if service.namespace != id.namespace {
            continue;
        }
        let ingress = format!("{}-ingress", service.name);
        let mesh = format!("{}-mesh", service.name);
        if id.name == service.name || id.name == ingress || id.name == mesh {
            targets.insert(service.clone());
            targets.extend(route_target(
                store,
                service.namespace.clone(),
                &service.name,
            ));
        }
    }
    targets
}

fn route_target(
    store: &ResourceStore,
    namespace: Option<String>,
    name: &str,
) -> Option<ResourceId> {
    let route = ResourceId::new(SERVING_API, "Route", namespace, name);
    store.contains(&route).then_some(route)
}

fn is_knative_target(id: &ResourceId) -> bool {
    matches!(
        id.kind.as_str(),
        "Service" | "Route" | "Revision" | "Configuration"
    )
}

fn add_destination_targets(
    graph: &mut GraphBuilder,
    id: &ResourceId,
    object: &kube::api::DynamicObject,
    resolver: &HostResolver<'_>,
    hosts: &[String],
) {
    let Some(http) = field(object, &["spec", "http"]).and_then(Value::as_array) else {
        return;
    };
    for (http_index, entry) in http.iter().enumerate() {
        let Some(routes) = entry.get("route").and_then(Value::as_array) else {
            continue;
        };
        for (route_index, route) in routes.iter().enumerate() {
            let Some(destination) = route.get("destination") else {
                continue;
            };
            let Some(host) = destination.get("host").and_then(Value::as_str) else {
                continue;
            };
            let Some(host) = host_from_url(host) else {
                continue;
            };
            let weight = destination.get("weight").and_then(Value::as_u64);
            let subset = destination.get("subset").and_then(Value::as_str);
            for target in resolver.resolve(&host, id.namespace.as_deref()) {
                if !graph.contains_node(&target) {
                    continue;
                }
                let mut edge = new_edge(EdgeKind::Targets, id, &target);
                insert_text_metadata(&mut edge, "destination_host", &host);
                insert_metadata(&mut edge, "http_index", json!(http_index));
                if let Some(weight) = weight {
                    insert_metadata(&mut edge, "weight", json!(weight));
                }
                if let Some(subset) = subset {
                    insert_text_metadata(&mut edge, "subset", subset);
                }
                insert_metadata(&mut edge, "hosts", json!(hosts));
                let suffix = format!("{http_index}:{route_index}");
                add_unique_edge(graph, edge, Some(&suffix));
            }
        }
    }
}

fn add_destination_rule_edges(
    graph: &mut GraphBuilder,
    id: &ResourceId,
    object: &kube::api::DynamicObject,
    resolver: &HostResolver<'_>,
) {
    let Some(host) = text_field(object, &["spec", "host"]).and_then(host_from_url) else {
        return;
    };
    let subsets: Vec<Value> = field(object, &["spec", "subsets"])
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let subset_names: Vec<&str> = subsets
        .iter()
        .filter_map(|subset| subset.get("name").and_then(Value::as_str))
        .collect();
    let tls_mode = text_field(object, &["spec", "trafficPolicy", "tls", "mode"]);
    for target in resolver.resolve(&host, id.namespace.as_deref()) {
        if !graph.contains_node(&target) {
            continue;
        }
        let mut edge = new_edge(EdgeKind::Targets, id, &target);
        insert_text_metadata(&mut edge, "host", &host);
        insert_metadata(&mut edge, "subsets", json!(subset_names));
        if let Some(mode) = tls_mode {
            insert_text_metadata(&mut edge, "mtls_mode", mode);
        }
        add_unique_edge(graph, edge, None);
    }
}
