//! policy.istio correlation stage.
//!
//! Correlates Istio VirtualServices, DestinationRules, PeerAuthentications,
//! and AuthorizationPolicies to the Knative Service, Route, and Revision graph.

use std::collections::BTreeMap;

use knect_proto::graph::{Badge, Health, Severity, Warning};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::Value;

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{NodeSpec, field, ready_health, resource_node, text_field};
use crate::store::ResourceStore;

use super::support::{
    HostIndex, ISTIO_NETWORKING_GROUP, ISTIO_SECURITY_GROUP, ids_by_group_kind, knative_host_index,
    managed_origin, vs_hosts,
};

mod security;
mod traffic;

/// Correlates Istio policy and traffic configuration.
#[derive(Debug, Default, Clone, Copy)]
pub struct IstioPolicyCorrelator;

/// One host listed by a custom VirtualService that conflicts with Knative.
#[derive(Debug, Clone)]
pub(crate) struct HostConflict {
    pub(crate) host: String,
    pub(crate) targets: Vec<ResourceId>,
}

/// Conflicts keyed by the custom VirtualService identity.
pub(crate) type Conflicts = BTreeMap<ResourceId, Vec<HostConflict>>;

impl Correlator for IstioPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.istio"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let store = &ctx.store;
        let virtual_services = ids_by_group_kind(store, ISTIO_NETWORKING_GROUP, "VirtualService");
        let destination_rules = ids_by_group_kind(store, ISTIO_NETWORKING_GROUP, "DestinationRule");
        let peer_authentications =
            ids_by_group_kind(store, ISTIO_SECURITY_GROUP, "PeerAuthentication");
        let authorization_policies =
            ids_by_group_kind(store, ISTIO_SECURITY_GROUP, "AuthorizationPolicy");
        let hosts = knative_host_index(store);
        let conflicts = collect_conflicts(store, &virtual_services, &hosts);

        add_virtual_service_nodes(store, graph, &virtual_services, &conflicts);
        add_destination_rule_nodes(store, graph, &destination_rules);
        security::add_security_nodes(store, graph, &peer_authentications, &authorization_policies);

        traffic::add_traffic_edges(
            store,
            graph,
            TrafficInputs {
                virtual_services: &virtual_services,
                destination_rules: &destination_rules,
                hosts: &hosts,
                conflicts: &conflicts,
            },
        );
        security::add_security_edges(store, graph, &peer_authentications, &authorization_policies);
        add_conflict_warnings(graph, &conflicts);
        Ok(())
    }
}

struct TrafficInputs<'a> {
    virtual_services: &'a [ResourceId],
    destination_rules: &'a [ResourceId],
    hosts: &'a HostIndex,
    conflicts: &'a Conflicts,
}

fn collect_conflicts(
    store: &ResourceStore,
    virtual_services: &[ResourceId],
    hosts: &HostIndex,
) -> Conflicts {
    let mut conflicts = Conflicts::new();
    for id in virtual_services {
        let Some(object) = store.get(id) else {
            continue;
        };
        if managed_origin(&object) != "custom" {
            continue;
        }
        for host in vs_hosts(&object) {
            let Some(targets) = hosts.get(&host) else {
                continue;
            };
            let targets: Vec<ResourceId> = targets
                .iter()
                .filter(|target| target.kind == "Route" || target.kind == "Service")
                .cloned()
                .collect();
            if targets.is_empty() {
                continue;
            }
            conflicts
                .entry(id.clone())
                .or_default()
                .push(HostConflict { host, targets });
        }
    }
    conflicts
}

fn add_virtual_service_nodes(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    ids: &[ResourceId],
    conflicts: &Conflicts,
) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let origin = managed_origin(&object);
        let host_conflicts = conflicts.get(id).map_or(&[][..], Vec::as_slice);
        let hosts = vs_hosts(&object);
        let routes = field(&object, &["spec", "http"])
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut summary = format!("{} hosts, {routes} http routes", hosts.len());
        if !host_conflicts.is_empty() {
            summary.push_str(", host conflict");
        }
        let mut badges = vec![Badge {
            label: origin.to_owned(),
            severity: Severity::Info,
            tooltip: None,
        }];
        if !host_conflicts.is_empty() {
            let hosts: Vec<&str> = host_conflicts
                .iter()
                .map(|conflict| conflict.host.as_str())
                .collect();
            badges.push(Badge {
                label: "host-conflict".to_owned(),
                severity: Severity::High,
                tooltip: Some(hosts.join(", ")),
            });
        }
        let mut node = resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::VirtualService,
                health: ready_health(&object, Health::Healthy),
                summary,
                badges,
            },
        );
        node.annotations
            .insert("knect.dev/managed-by".to_owned(), origin.to_owned());
        graph.add_node(node);
    }
}

fn add_destination_rule_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        let host = text_field(&object, &["spec", "host"]).unwrap_or("no host");
        let subsets: Vec<&str> = field(&object, &["spec", "subsets"])
            .and_then(Value::as_array)
            .map(|subsets| {
                subsets
                    .iter()
                    .filter_map(|subset| subset.get("name").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let tls_mode = text_field(&object, &["spec", "trafficPolicy", "tls", "mode"]);
        let mut summary = format!("{host}, {} subsets", subsets.len());
        if let Some(mode) = tls_mode {
            summary.push_str(&format!(", {mode}"));
        }
        let mut badges = Vec::new();
        if !subsets.is_empty() {
            badges.push(Badge {
                label: "subsets".to_owned(),
                severity: Severity::Info,
                tooltip: Some(subsets.join(", ")),
            });
        }
        if let Some(mode) = tls_mode {
            badges.push(Badge {
                label: "mtls".to_owned(),
                severity: Severity::Info,
                tooltip: Some(mode.to_owned()),
            });
        }
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::DestinationRule,
                health: ready_health(&object, Health::Healthy),
                summary,
                badges,
            },
        ));
    }
}

fn add_conflict_warnings(graph: &mut GraphBuilder, conflicts: &Conflicts) {
    for (virtual_service, entries) in conflicts {
        for conflict in entries {
            let resource = conflict
                .targets
                .iter()
                .find(|target| target.kind == "Route")
                .or_else(|| conflict.targets.first());
            let target = resource.map_or_else(
                || "Knative resource".to_owned(),
                |target| format!("{} {}", target.kind, target.name),
            );
            graph.add_warning(Warning {
                code: "istio_host_conflict".to_owned(),
                message: format!(
                    "custom VirtualService {} host {} conflicts with {target}",
                    virtual_service.name, conflict.host
                ),
                severity: Severity::High,
                resource: resource.cloned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::support::condition_status;
    use crate::store::resource::tests::test_object;

    #[test]
    fn only_custom_virtual_services_are_conflict_candidates() {
        let mut store = ResourceStore::new();
        let mut managed = test_object(
            "networking.istio.io/v1",
            "VirtualService",
            Some("default"),
            "order-ingress",
        );
        managed.metadata.labels = Some(
            [("serving.knative.dev/route".to_owned(), "order".to_owned())]
                .into_iter()
                .collect(),
        );
        managed.data = serde_json::json!({
            "spec": { "hosts": ["order.default.svc.cluster.local"] }
        });
        store.upsert(managed);
        let mut custom = test_object(
            "networking.istio.io/v1",
            "VirtualService",
            Some("default"),
            "order-custom",
        );
        custom.data = serde_json::json!({
            "spec": { "hosts": ["order.default.svc.cluster.local"] }
        });
        store.upsert(custom);

        let hosts: HostIndex = [(
            "order.default.svc.cluster.local".to_owned(),
            [ResourceId::namespaced(
                "serving.knative.dev/v1",
                "Route",
                "default",
                "order",
            )]
            .into_iter()
            .collect(),
        )]
        .into_iter()
        .collect();
        let ids = ids_by_group_kind(&store, ISTIO_NETWORKING_GROUP, "VirtualService");
        let conflicts = collect_conflicts(&store, &ids, &hosts);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts.contains_key(&ResourceId::namespaced(
            "networking.istio.io/v1",
            "VirtualService",
            "default",
            "order-custom"
        )));
    }

    #[test]
    fn condition_status_is_read_for_health() {
        let mut object = test_object(
            "networking.istio.io/v1",
            "VirtualService",
            Some("default"),
            "order",
        );
        object.data = serde_json::json!({
            "status": { "conditions": [{ "type": "Ready", "status": "False" }] }
        });
        assert_eq!(condition_status(&object, "Ready"), Some("False"));
    }
}
