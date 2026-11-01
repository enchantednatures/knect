//! Projection of scan results into graph nodes, edges, and warnings.

use knect_proto::graph::Health;
use knect_proto::links::LinkRecord;
use knect_proto::{NodeCategory, ResourceId};
use serde_json::json;

use crate::correlate::CorrelationContext;
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, insert_metadata, new_edge, ready_health, resource_node,
};
use crate::links::candidate::{LinkScan, link_via_key};
use crate::links::index::api_group;

/// Adds missing target nodes, then projects candidates into edges.
///
/// A target node is only created when the object already exists in the store;
/// unresolved references stay warnings and never invent placeholder nodes.
pub(crate) fn apply_scan(
    ctx: &CorrelationContext,
    graph: &mut GraphBuilder,
    scan: LinkScan,
) -> Result<Vec<LinkRecord>, CoreError> {
    for warning in scan.warnings() {
        graph.add_warning(warning);
    }
    let mut records = Vec::new();
    for candidate in scan.candidates() {
        ensure_target_node(ctx, graph, &candidate.target);
        let health = target_health(graph, &candidate.target);
        let record = candidate.record(health);
        let mut edge = new_edge(candidate.kind, &candidate.source, &candidate.target);
        edge.health = health;
        edge.label = Some(link_via_key(candidate.via).to_owned());
        insert_metadata(&mut edge, "via", json!(link_via_key(candidate.via)));
        insert_metadata(&mut edge, "link", serde_json::to_value(&record)?);
        insert_metadata(
            &mut edge,
            "evidence",
            serde_json::to_value(&record.evidence)?,
        );
        add_unique_edge(graph, edge, Some(link_via_key(candidate.via)));
        records.push(record);
    }
    Ok(records)
}

fn ensure_target_node(ctx: &CorrelationContext, graph: &mut GraphBuilder, id: &ResourceId) {
    if graph.contains_node(id) {
        return;
    }
    let Some(object) = ctx.store.get(id) else {
        return;
    };
    let category = category_for(id);
    let health = ready_health(&object, Health::Unknown);
    let summary = summary_for(id, category);
    graph.add_node(resource_node(
        &object,
        NodeSpec {
            category,
            health,
            summary,
            badges: Vec::new(),
        },
    ));
}

fn target_health(graph: &GraphBuilder, target: &ResourceId) -> Health {
    match graph.node(target).map(|node| node.health) {
        Some(Health::Healthy) => Health::Healthy,
        _ => Health::Unknown,
    }
}

fn category_for(id: &ResourceId) -> NodeCategory {
    match (api_group(&id.api_version), id.kind.as_str()) {
        ("", "Service") => NodeCategory::Service,
        ("", "Endpoints") => NodeCategory::Endpoints,
        ("", "Secret") => NodeCategory::Secret,
        ("", "ConfigMap") => NodeCategory::ConfigMap,
        ("serving.knative.dev", "Service") => NodeCategory::KService,
        ("serving.knative.dev", "Route") => NodeCategory::Route,
        ("serving.knative.dev", "Configuration") => NodeCategory::Configuration,
        ("serving.knative.dev", "Revision") => NodeCategory::Revision,
        ("eventing.knative.dev", "Broker") => NodeCategory::Broker,
        ("eventing.knative.dev", "Trigger") => NodeCategory::Trigger,
        ("sources.knative.dev", "SinkBinding") => NodeCategory::SinkBinding,
        ("sources.knative.dev", _) => NodeCategory::Source,
        (_, "Topic") => NodeCategory::Topic,
        _ => NodeCategory::BackingService,
    }
}

fn summary_for(id: &ResourceId, category: NodeCategory) -> String {
    match category {
        NodeCategory::Secret => "secret (values redacted)".to_owned(),
        NodeCategory::ConfigMap => "config map (values redacted)".to_owned(),
        NodeCategory::Service => format!("service {}", id.name),
        NodeCategory::Endpoints => format!("endpoints {}", id.name),
        _ => format!("{} {}", id.kind, id.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_cover_core_and_operator_targets() {
        assert_eq!(
            category_for(&ResourceId::namespaced("v1", "Service", "default", "redis")),
            NodeCategory::Service
        );
        assert_eq!(
            category_for(&ResourceId::namespaced("v1", "Secret", "default", "creds")),
            NodeCategory::Secret
        );
        assert_eq!(
            category_for(&ResourceId::namespaced(
                "redis.redis.opstreelabs.in/v1beta2",
                "RedisCluster",
                "default",
                "redis"
            )),
            NodeCategory::BackingService
        );
    }

    #[test]
    fn summaries_never_include_values() {
        let id = ResourceId::namespaced("v1", "Secret", "default", "creds");
        let summary = summary_for(&id, NodeCategory::Secret);
        assert!(summary.contains("redacted"));
        assert!(!summary.contains("creds"));
    }
}
