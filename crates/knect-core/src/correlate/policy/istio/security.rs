//! Istio security edges: mTLS and authorization policies.

use std::collections::BTreeSet;

use knect_proto::graph::{Badge, EdgeKind, Health, Severity};
use knect_proto::{NodeCategory, ResourceId};
use kube::ResourceExt;
use serde_json::{Value, json};

use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge,
    ready_health, resource_node, text_field,
};
use crate::store::ResourceStore;

use super::super::support::{
    SERVING_API, SERVING_GROUP, ids_by_group_kind, object_matches, selector_labels,
};

pub(super) fn add_security_nodes(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    peer_authentications: &[ResourceId],
    authorization_policies: &[ResourceId],
) {
    for id in peer_authentications {
        let Some(object) = store.get(id) else {
            continue;
        };
        let mode = text_field(&object, &["spec", "mtls", "mode"]).unwrap_or("UNSET");
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::PeerAuthentication,
                health: ready_health(&object, Health::Healthy),
                summary: format!("mTLS {mode}"),
                badges: vec![Badge {
                    label: format!("mtls-{}", mode.to_ascii_lowercase()),
                    severity: mtls_severity(mode),
                    tooltip: Some(mode.to_owned()),
                }],
            },
        ));
    }
    for id in authorization_policies {
        let Some(object) = store.get(id) else {
            continue;
        };
        let action = text_field(&object, &["spec", "action"]).unwrap_or("ALLOW");
        let rules = field(&object, &["spec", "rules"])
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::AuthorizationPolicy,
                health: ready_health(&object, Health::Healthy),
                summary: format!("{action}, {rules} rules"),
                badges: vec![Badge {
                    label: format!("action-{}", action.to_ascii_lowercase()),
                    severity: Severity::Info,
                    tooltip: Some(format!("{action} authorization policy")),
                }],
            },
        ));
    }
}

fn mtls_severity(mode: &str) -> Severity {
    match mode {
        "DISABLE" => Severity::Medium,
        "PERMISSIVE" => Severity::Low,
        _ => Severity::Info,
    }
}

pub(super) fn add_security_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    peer_authentications: &[ResourceId],
    authorization_policies: &[ResourceId],
) {
    for id in peer_authentications {
        add_policy_edges(store, graph, id, SecurityKind::PeerAuthentication);
    }
    for id in authorization_policies {
        add_policy_edges(store, graph, id, SecurityKind::AuthorizationPolicy);
    }
}

#[derive(Debug, Clone, Copy)]
enum SecurityKind {
    PeerAuthentication,
    AuthorizationPolicy,
}

impl SecurityKind {
    const fn name(self) -> &'static str {
        match self {
            Self::PeerAuthentication => "PeerAuthentication",
            Self::AuthorizationPolicy => "AuthorizationPolicy",
        }
    }
}

fn add_policy_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    id: &ResourceId,
    kind: SecurityKind,
) {
    let Some(object) = store.get(id) else {
        return;
    };
    let selector = selector_labels(&object);
    let scope = if selector.is_empty() {
        "namespace"
    } else {
        "selector"
    };
    for target in policy_targets(store, id, &selector) {
        if !graph.contains_node(&target) {
            continue;
        }
        let mut edge = new_edge(EdgeKind::ProtectedBy, &target, id);
        insert_text_metadata(&mut edge, "policy_kind", kind.name());
        insert_text_metadata(&mut edge, "scope", scope);
        insert_metadata(&mut edge, "selector", json!(selector));
        if let Some(namespace) = &id.namespace {
            insert_text_metadata(&mut edge, "namespace", namespace);
        }
        match kind {
            SecurityKind::PeerAuthentication => {
                let mode = text_field(&object, &["spec", "mtls", "mode"]).unwrap_or("UNSET");
                insert_text_metadata(&mut edge, "mtls_mode", mode);
            }
            SecurityKind::AuthorizationPolicy => {
                let action = text_field(&object, &["spec", "action"]).unwrap_or("ALLOW");
                let rules = field(&object, &["spec", "rules"])
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                insert_text_metadata(&mut edge, "action", action);
                insert_metadata(&mut edge, "rules_count", json!(rules));
            }
        }
        add_unique_edge(graph, edge, None);
    }
}

fn policy_targets(
    store: &ResourceStore,
    id: &ResourceId,
    selector: &std::collections::BTreeMap<String, String>,
) -> BTreeSet<ResourceId> {
    let mut targets = BTreeSet::new();
    let services = ids_by_group_kind(store, SERVING_GROUP, "Service");
    let routes = ids_by_group_kind(store, SERVING_GROUP, "Route");
    for service_id in services {
        if service_id.namespace != id.namespace {
            continue;
        }
        let Some(service) = store.get(&service_id) else {
            continue;
        };
        let matches = selector.is_empty() || object_matches(&service, selector);
        if !matches {
            continue;
        }
        targets.insert(service_id.clone());
        if selector.is_empty() {
            targets.extend(route_target(
                store,
                id.namespace.as_deref(),
                &service_id.name,
            ));
            continue;
        }
        targets.extend(route_target(
            store,
            id.namespace.as_deref(),
            &service_id.name,
        ));
        targets.extend(revisions_for_service(
            store,
            id.namespace.as_deref(),
            &service_id.name,
        ));
    }
    if !selector.is_empty() {
        for route_id in routes {
            if route_id.namespace != id.namespace {
                continue;
            }
            if let Some(route) = store.get(&route_id)
                && object_matches(&route, selector)
            {
                targets.insert(route_id);
            }
        }
    }
    if selector.is_empty() {
        targets.extend(
            ids_by_group_kind(store, SERVING_GROUP, "Revision")
                .into_iter()
                .filter(|revision| revision.namespace == id.namespace),
        );
    }
    targets
}

fn route_target(store: &ResourceStore, namespace: Option<&str>, name: &str) -> Option<ResourceId> {
    let id = ResourceId::new(SERVING_API, "Route", namespace.map(ToOwned::to_owned), name);
    store.contains(&id).then_some(id)
}

fn revisions_for_service(
    store: &ResourceStore,
    namespace: Option<&str>,
    service: &str,
) -> Vec<ResourceId> {
    ids_by_group_kind(store, SERVING_GROUP, "Revision")
        .into_iter()
        .filter(|revision| revision.namespace.as_deref() == namespace)
        .filter(|revision| {
            store.get(revision).is_some_and(|object| {
                object
                    .labels()
                    .get("serving.knative.dev/service")
                    .is_some_and(|owner| owner == service)
            })
        })
        .collect()
}
