//! Knative Serving correlation.
//!
//! Builds the KService -> Route/Configuration -> Revision -> PodAutoscaler ->
//! Deployment -> Pod chain and the traffic edges between a Route and the
//! Revisions it routes to. The correlator is pure: every input comes from the
//! sanitized [`crate::store::ResourceStore`] snapshot.

use std::collections::{BTreeMap, BTreeSet};

use knect_proto::graph::{Badge, EdgeKind, Health, Severity};
use knect_proto::health::ScaleState;
use knect_proto::{Gvk, NodeCategory, ResourceId};
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::{Value, json};

pub use crate::graph::edge_id;

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, condition_message, condition_reason, condition_status,
    conditions_contain, field, insert_metadata, namespaced, new_edge, object_name, ready_health,
    resource_node, status_health, text_field, u64_field, warning,
};
use crate::store::{ResourceStore, matches_labels};

const SERVING_GROUP: &str = "serving.knative.dev";
const SERVING_VERSION: &str = "v1";
const SERVING_API: &str = "serving.knative.dev/v1";
const CONFIGURATION_LABEL: &str = "serving.knative.dev/configuration";

/// Correlates Knative Serving resources into the graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct ServingCorrelator;

impl Correlator for ServingCorrelator {
    fn name(&self) -> &'static str {
        "serving"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let store = &ctx.store;
        let services = ids_for(store, "Service");
        let routes = ids_for(store, "Route");
        let configurations = ids_for(store, "Configuration");
        let revisions = ids_for(store, "Revision");
        let mut autoscalers =
            store.ids_by_group_kind("autoscaling.internal.knative.dev", "PodAutoscaler");
        autoscalers.extend(store.ids_by_group_kind("autoscaling.knative.dev", "PodAutoscaler"));
        autoscalers.sort();
        autoscalers.dedup();
        let deployments = ids_for_gvk(store, Gvk::new("apps", "v1", "Deployment"));
        let pods = ids_for_gvk(store, Gvk::new("", "v1", "Pod"));

        add_service_nodes(store, graph, &services);
        add_route_nodes(store, graph, &routes);
        add_configuration_nodes(store, graph, &configurations);
        add_revision_nodes(store, graph, &revisions);
        add_autoscaler_nodes(store, graph, &autoscalers);
        add_deployment_nodes(store, graph, &deployments);
        add_pod_nodes(store, graph, &pods);

        add_service_ownership(
            store,
            graph,
            ServiceOwnership {
                children: &routes,
                services: &services,
            },
        );
        add_service_ownership(
            store,
            graph,
            ServiceOwnership {
                children: &configurations,
                services: &services,
            },
        );
        add_route_traffic(store, graph, &routes);
        add_configuration_revisions(store, graph, &revisions);
        add_autoscaler_edges(
            store,
            graph,
            AutoscalerTargets {
                autoscalers: &autoscalers,
                revisions: &revisions,
            },
        );
        add_deployment_pods(store, graph, &deployments);

        Ok(())
    }
}

fn ids_for(store: &ResourceStore, kind: &str) -> Vec<ResourceId> {
    ids_for_gvk(store, Gvk::new(SERVING_GROUP, SERVING_VERSION, kind))
}

fn ids_for_gvk(store: &ResourceStore, gvk: Gvk) -> Vec<ResourceId> {
    store.ids_by_gvk(&gvk)
}

fn skip_malformed(graph: &mut GraphBuilder, id: &ResourceId, object: &DynamicObject) -> bool {
    if object_name(object).is_some() {
        return false;
    }
    graph.add_warning(warning(
        "malformed_object",
        format!("{id} has no metadata.name"),
        Some(id.clone()),
    ));
    true
}

fn add_service_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let health = ready_health(&object, Health::Unknown);
        let summary = text_field(&object, &["status", "url"])
            .map(ToOwned::to_owned)
            .or_else(|| {
                text_field(&object, &["status", "latestReadyRevisionName"])
                    .map(|revision| format!("latest ready revision {revision}"))
            })
            .unwrap_or_else(|| "no URL".to_owned());
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::KService,
                health,
                summary,
                badges: Vec::new(),
            },
        ));
    }
}

fn add_route_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let health = ready_health(&object, Health::Unknown);
        let summary = traffic_summary(traffic_entries(&object));
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Route,
                health,
                summary,
                badges: Vec::new(),
            },
        ));
    }
}

fn add_configuration_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let health = ready_health(&object, Health::Unknown);
        let summary = text_field(&object, &["status", "latestCreatedRevisionName"])
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "no revisions".to_owned());
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Configuration,
                health,
                summary,
                badges: Vec::new(),
            },
        ));
    }
}

fn add_revision_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let actual = u64_field(&object, &["status", "actualReplicas"]).unwrap_or(0);
        let desired = u64_field(&object, &["status", "desiredReplicas"]).unwrap_or(0);
        let resource_status = condition_status(&object, "ResourcesAvailable");
        let resource_reason = condition_reason(&object, "ResourcesAvailable");
        let blocking =
            resource_status == Some("False") && resource_reason.is_some_and(is_blocking_reason);
        let (scale_state, health) = revision_scale(RevisionSignals {
            ready: condition_status(&object, "Ready"),
            active: condition_status(&object, "Active"),
            actual,
            desired,
            blocking,
        });
        let mut summary = format!("{actual}/{desired} replicas");
        if health != Health::Healthy
            && let Some(reason) = first_unhealthy_reason(&object)
        {
            summary.push_str(&format!(", {reason}"));
        }
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Revision,
                health,
                summary,
                badges: revision_badges(&object, scale_state, health),
            },
        ));
    }
}

fn add_autoscaler_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let class = object
            .annotations()
            .get("autoscaling.knative.dev/class")
            .map_or("KPA", String::as_str);
        let actual = u64_field(&object, &["status", "actualScale"]).unwrap_or(0);
        let desired = u64_field(&object, &["status", "desiredScale"]).unwrap_or(0);
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::PodAutoscaler,
                health: ready_health(&object, Health::Unknown),
                summary: format!("{class} desired={desired} actual={actual}"),
                badges: Vec::new(),
            },
        ));
    }
}

fn add_deployment_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let ready = u64_field(&object, &["status", "readyReplicas"]).unwrap_or(0);
        let desired = u64_field(&object, &["spec", "replicas"]).unwrap_or(0);
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Deployment,
                health: status_health(condition_status(&object, "Available"), Health::Unknown),
                summary: format!("{ready}/{desired} replicas"),
                badges: Vec::new(),
            },
        ));
    }
}

fn add_pod_nodes(store: &ResourceStore, graph: &mut GraphBuilder, ids: &[ResourceId]) {
    for id in ids {
        let Some(object) = store.get(id) else {
            continue;
        };
        if skip_malformed(graph, id, &object) {
            continue;
        }
        let phase = text_field(&object, &["status", "phase"]).unwrap_or("Unknown");
        let restarts = pod_restarts(&object);
        graph.add_node(resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Pod,
                health: pod_health(&object),
                summary: format!("{phase}, {restarts} restarts"),
                badges: Vec::new(),
            },
        ));
    }
}

struct ServiceOwnership<'a> {
    children: &'a [ResourceId],
    services: &'a [ResourceId],
}

fn add_service_ownership(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    ownership: ServiceOwnership<'_>,
) {
    for child_id in ownership.children {
        let Some(object) = store.get(child_id) else {
            continue;
        };
        let Some(service_id) = owner_in_candidates(
            &object,
            child_id,
            OwnerQuery {
                kind: "Service",
                candidates: ownership.services,
                same_name_fallback: true,
            },
        ) else {
            continue;
        };
        let edge = new_edge(EdgeKind::Owns, &service_id, child_id);
        add_unique_edge(graph, edge, None);
    }
}

fn add_route_traffic(store: &ResourceStore, graph: &mut GraphBuilder, routes: &[ResourceId]) {
    for route_id in routes {
        let Some(route) = store.get(route_id) else {
            continue;
        };
        for (position, entry) in traffic_entries(&route).iter().enumerate() {
            let Some(revision_name) = entry.get("revisionName").and_then(Value::as_str) else {
                continue;
            };
            let revision_id = namespaced(
                SERVING_API,
                "Revision",
                route_id.namespace.as_deref(),
                revision_name,
            );
            if !graph.contains_node(&revision_id) {
                graph.add_warning(warning(
                    "missing_revision",
                    format!(
                        "route {} references missing revision {revision_name}",
                        route_id.name
                    ),
                    Some(route_id.clone()),
                ));
                continue;
            }
            let percent = entry.get("percent").and_then(Value::as_u64);
            let tag = entry.get("tag").and_then(Value::as_str);
            let latest_revision = entry
                .get("latestRevision")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let url = entry.get("url").and_then(Value::as_str);
            let label = match (percent, tag) {
                (Some(percent), Some(tag)) => format!("{percent}% {tag}"),
                (Some(percent), None) => format!("{percent}%"),
                (None, Some(tag)) => tag.to_owned(),
                (None, None) => revision_name.to_owned(),
            };
            let mut edge = new_edge(EdgeKind::RoutesTo, route_id, &revision_id);
            edge.label = Some(label);
            if let Some(percent) = percent {
                insert_metadata(&mut edge, "percent", json!(percent));
            }
            if let Some(tag) = tag {
                insert_metadata(&mut edge, "tag", json!(tag));
            }
            insert_metadata(&mut edge, "latest_revision", json!(latest_revision));
            if let Some(url) = url {
                insert_metadata(&mut edge, "url", json!(url));
            }
            let discriminator = tag.map_or_else(|| position.to_string(), ToOwned::to_owned);
            add_unique_edge(graph, edge, Some(&discriminator));
        }
    }
}

fn add_configuration_revisions(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    revisions: &[ResourceId],
) {
    let configurations = ids_for(store, "Configuration");
    for revision_id in revisions {
        let Some(revision) = store.get(revision_id) else {
            continue;
        };
        let from_label = revision.labels().get(CONFIGURATION_LABEL).map(|name| {
            namespaced(
                SERVING_API,
                "Configuration",
                revision_id.namespace.as_deref(),
                name,
            )
        });
        let configuration_id = from_label.filter(|id| {
            graph
                .node(id)
                .is_some_and(|node| node.category == NodeCategory::Configuration)
        });
        let configuration_id = configuration_id.or_else(|| {
            owner_in_candidates(
                &revision,
                revision_id,
                OwnerQuery {
                    kind: "Configuration",
                    candidates: &configurations,
                    same_name_fallback: false,
                },
            )
        });
        let Some(configuration_id) = configuration_id else {
            continue;
        };
        let edge = new_edge(EdgeKind::Owns, &configuration_id, revision_id);
        add_unique_edge(graph, edge, None);
    }
}

struct AutoscalerTargets<'a> {
    autoscalers: &'a [ResourceId],
    revisions: &'a [ResourceId],
}

fn add_autoscaler_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    targets: AutoscalerTargets<'_>,
) {
    for autoscaler_id in targets.autoscalers {
        let Some(autoscaler) = store.get(autoscaler_id) else {
            continue;
        };
        match owner_in_candidates(
            &autoscaler,
            autoscaler_id,
            OwnerQuery {
                kind: "Revision",
                candidates: targets.revisions,
                same_name_fallback: true,
            },
        ) {
            Some(revision_id) => {
                let edge = new_edge(EdgeKind::Scales, &revision_id, autoscaler_id);
                add_unique_edge(graph, edge, None);
            }
            None => graph.add_warning(warning(
                "missing_revision",
                format!(
                    "PodAutoscaler {} has no matching Revision",
                    autoscaler_id.name
                ),
                Some(autoscaler_id.clone()),
            )),
        }

        let target = field(&autoscaler, &["spec", "scaleTargetRef"])
            .or_else(|| field(&autoscaler, &["status", "scaleTargetRef"]))
            .and_then(|reference| {
                crate::graph::support::reference_id(reference, autoscaler_id.namespace.as_deref())
            });
        match target.filter(|id| is_deployment(graph, id)) {
            Some(deployment_id) => {
                let class = autoscaler
                    .annotations()
                    .get("autoscaling.knative.dev/class")
                    .map_or("KPA", String::as_str);
                let mut edge = new_edge(EdgeKind::Scales, autoscaler_id, &deployment_id);
                insert_metadata(&mut edge, "class", json!(class));
                insert_metadata(
                    &mut edge,
                    "desired_scale",
                    json!(u64_field(&autoscaler, &["status", "desiredScale"]).unwrap_or(0)),
                );
                insert_metadata(
                    &mut edge,
                    "actual_scale",
                    json!(u64_field(&autoscaler, &["status", "actualScale"]).unwrap_or(0)),
                );
                add_unique_edge(graph, edge, None);
            }
            None => graph.add_warning(warning(
                "missing_deployment",
                format!(
                    "PodAutoscaler {} has no matching Deployment",
                    autoscaler_id.name
                ),
                Some(autoscaler_id.clone()),
            )),
        }
    }
}

fn add_deployment_pods(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    deployments: &[ResourceId],
) {
    let pods = ids_for_gvk(store, Gvk::new("", "v1", "Pod"));
    for deployment_id in deployments {
        let Some(deployment) = store.get(deployment_id) else {
            continue;
        };
        let selector = label_selector(&deployment);
        let mut matching: BTreeSet<ResourceId> = BTreeSet::new();
        if let Some(uid) = deployment.uid() {
            for id in store.ids_owned_by_uid(&uid) {
                if id.kind == "Pod" {
                    matching.insert(id);
                }
            }
        }
        if !selector.is_empty() {
            for pod_id in &pods {
                let Some(pod) = store.get(pod_id) else {
                    continue;
                };
                if pod_id.namespace == deployment_id.namespace && matches_labels(&pod, &selector) {
                    matching.insert(pod_id.clone());
                }
            }
        }
        let ready = u64_field(&deployment, &["status", "readyReplicas"]).unwrap_or(0);
        let desired = u64_field(&deployment, &["spec", "replicas"]).unwrap_or(0);
        if matching.is_empty() && (ready > 0 || desired > 0) {
            graph.add_warning(warning(
                "missing_pod",
                format!("Deployment {} has no correlated Pods", deployment_id.name),
                Some(deployment_id.clone()),
            ));
        }
        for pod_id in matching {
            let edge = new_edge(EdgeKind::Runs, deployment_id, &pod_id);
            add_unique_edge(graph, edge, None);
        }
    }
}

struct OwnerQuery<'a> {
    kind: &'a str,
    candidates: &'a [ResourceId],
    same_name_fallback: bool,
}

fn owner_in_candidates(
    object: &DynamicObject,
    object_id: &ResourceId,
    query: OwnerQuery<'_>,
) -> Option<ResourceId> {
    for reference in object.owner_references() {
        if reference.kind != query.kind {
            continue;
        }
        if let Some(found) = query.candidates.iter().find(|candidate| {
            candidate.name == reference.name
                && candidate.namespace == object_id.namespace
                && (reference.api_version.is_empty()
                    || candidate.api_version == reference.api_version)
        }) {
            return Some(found.clone());
        }
    }
    if query.same_name_fallback {
        return query
            .candidates
            .iter()
            .find(|candidate| {
                candidate.name == object_id.name && candidate.namespace == object_id.namespace
            })
            .cloned();
    }
    None
}

fn is_deployment(graph: &GraphBuilder, id: &ResourceId) -> bool {
    graph
        .node(id)
        .is_some_and(|node| node.category == NodeCategory::Deployment)
}

fn traffic_entries(object: &DynamicObject) -> &[Value] {
    let status = field(object, &["status", "traffic"])
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !status.is_empty() {
        return status;
    }
    field(object, &["spec", "traffic"])
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn traffic_summary(entries: &[Value]) -> String {
    if entries.is_empty() {
        return "no traffic".to_owned();
    }
    let parts: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let revision = entry.get("revisionName").and_then(Value::as_str)?;
            let percent = entry.get("percent").and_then(Value::as_u64).unwrap_or(0);
            let tag = entry.get("tag").and_then(Value::as_str);
            Some(match tag {
                Some(tag) => format!("{percent}% {revision} ({tag})"),
                None => format!("{percent}% {revision}"),
            })
        })
        .collect();
    if parts.is_empty() {
        "no traffic".to_owned()
    } else {
        parts.join(", ")
    }
}

struct RevisionSignals<'a> {
    ready: Option<&'a str>,
    active: Option<&'a str>,
    actual: u64,
    desired: u64,
    blocking: bool,
}

fn revision_scale(signals: RevisionSignals<'_>) -> (ScaleState, Health) {
    let RevisionSignals {
        ready,
        active,
        actual,
        desired,
        blocking,
    } = signals;
    if ready == Some("False") {
        return (
            ScaleState::Degraded,
            if blocking {
                Health::Blocked
            } else {
                Health::Degraded
            },
        );
    }
    if ready == Some("Unknown") || active == Some("Unknown") {
        return (ScaleState::ScalingUp, Health::Progressing);
    }
    match active {
        Some("True") => (ScaleState::Active, Health::Healthy),
        Some("False") => (ScaleState::ScaledToZero, Health::Healthy),
        Some(_) => (ScaleState::ScalingUp, Health::Progressing),
        None if actual > 0 => (
            ScaleState::Active,
            if ready == Some("True") {
                Health::Healthy
            } else {
                Health::Unknown
            },
        ),
        None if desired > 0 => (ScaleState::ScalingUp, Health::Progressing),
        None => (ScaleState::Unknown, Health::Unknown),
    }
}

fn is_blocking_reason(reason: &str) -> bool {
    matches!(
        reason,
        "FailedCreate"
            | "Unschedulable"
            | "ContainerMissing"
            | "Forbidden"
            | "Unauthorized"
            | "QuotaExceeded"
            | "LimitExceeded"
            | "RevisionMissing"
            | "ConfigurationMissing"
    ) || reason.to_ascii_lowercase().starts_with("policy")
}

fn first_unhealthy_reason(object: &DynamicObject) -> Option<&str> {
    ["Ready", "ResourcesAvailable", "ContainerHealthy", "Active"]
        .iter()
        .find_map(|type_name| {
            let status = condition_status(object, type_name)?;
            if status == "True" {
                return None;
            }
            condition_reason(object, type_name)
        })
}

fn revision_badges(object: &DynamicObject, scale_state: ScaleState, health: Health) -> Vec<Badge> {
    let label = match scale_state {
        ScaleState::Active => "active",
        ScaleState::ScalingUp => "scaling-up",
        ScaleState::ScaledToZero => "scaled-to-zero",
        ScaleState::Degraded => "degraded",
        ScaleState::Unknown => "unknown",
    };
    let mut badges = vec![Badge {
        label: label.to_owned(),
        severity: Severity::Info,
        tooltip: if scale_state == ScaleState::ScaledToZero {
            Some("0K".to_owned())
        } else {
            None
        },
    }];
    if health == Health::Blocked {
        badges.push(Badge {
            label: "blocked".to_owned(),
            severity: Severity::Critical,
            tooltip: first_unhealthy_reason(object).map(ToOwned::to_owned),
        });
    }
    if conditions_contain(object, &["crashloop", "crash loop", "back-off restarting"]) {
        badges.push(Badge {
            label: "crashloop".to_owned(),
            severity: Severity::High,
            tooltip: condition_message(object, "ContainerHealthy").map(ToOwned::to_owned),
        });
    }
    if conditions_contain(object, &["imagepullbackoff", "errimagepull", "image pull"]) {
        badges.push(Badge {
            label: "image-pull".to_owned(),
            severity: Severity::High,
            tooltip: condition_message(object, "ContainerHealthy").map(ToOwned::to_owned),
        });
    }
    badges
}

fn pod_health(object: &DynamicObject) -> Health {
    let ready = condition_status(object, "Ready");
    match text_field(object, &["status", "phase"]) {
        Some("Running") => {
            if ready == Some("False") || ready == Some("Unknown") {
                status_health(ready, Health::Healthy)
            } else {
                Health::Healthy
            }
        }
        Some("Succeeded") => Health::Healthy,
        Some("Pending") => Health::Progressing,
        Some("Failed") => Health::Degraded,
        _ => status_health(ready, Health::Unknown),
    }
}

fn pod_restarts(object: &DynamicObject) -> u64 {
    field(object, &["status", "containerStatuses"])
        .and_then(Value::as_array)
        .map(|statuses| {
            statuses
                .iter()
                .filter_map(|status| status.get("restartCount").and_then(Value::as_u64))
                .sum()
        })
        .unwrap_or(0)
}

fn label_selector(object: &DynamicObject) -> BTreeMap<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(
        ready: Option<&str>,
        active: Option<&str>,
        actual: u64,
        desired: u64,
        blocking: bool,
    ) -> (ScaleState, Health) {
        revision_scale(RevisionSignals {
            ready,
            active,
            actual,
            desired,
            blocking,
        })
    }

    #[test]
    fn scale_states_cover_every_revision_shape() {
        assert_eq!(
            signals(Some("True"), Some("True"), 1, 1, false),
            (ScaleState::Active, Health::Healthy)
        );
        assert_eq!(
            signals(Some("True"), Some("False"), 0, 0, false),
            (ScaleState::ScaledToZero, Health::Healthy)
        );
        assert_eq!(
            signals(Some("Unknown"), Some("Unknown"), 0, 1, false),
            (ScaleState::ScalingUp, Health::Progressing)
        );
        assert_eq!(
            signals(Some("False"), Some("False"), 0, 1, false),
            (ScaleState::Degraded, Health::Degraded)
        );
        assert_eq!(
            signals(Some("False"), Some("False"), 0, 1, true),
            (ScaleState::Degraded, Health::Blocked)
        );
        assert_eq!(
            signals(Some("True"), None, 1, 1, false),
            (ScaleState::Active, Health::Healthy)
        );
        assert_eq!(
            signals(Some("True"), None, 0, 1, false),
            (ScaleState::ScalingUp, Health::Progressing)
        );
        assert_eq!(
            signals(Some("True"), None, 0, 0, false),
            (ScaleState::Unknown, Health::Unknown)
        );
    }

    #[test]
    fn blocking_reasons_are_policy_and_scheduling_failures() {
        for reason in [
            "FailedCreate",
            "Unschedulable",
            "ContainerMissing",
            "Forbidden",
            "Unauthorized",
            "QuotaExceeded",
            "policy-violation",
        ] {
            assert!(is_blocking_reason(reason), "{reason} must block");
        }
        assert!(!is_blocking_reason("ProgressDeadlineExceeded"));
        assert!(!is_blocking_reason("ExitCode1"));
    }
}
