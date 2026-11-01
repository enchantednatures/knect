//! Traffic rows and owning Route discovered from a topology.

use knect_proto::ResourceId;
use knect_proto::graph::{EdgeKind, NodeCategory, Topology};

use super::traffic::TrafficRow;

/// Traffic rows and owning Route discovered from a topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficSeed {
    /// Selected Knative Service.
    pub service: ResourceId,
    /// Owning Route, when present.
    pub route: Option<ResourceId>,
    /// Current traffic rows.
    pub rows: Vec<TrafficRow>,
}

/// Resolves the Service and Route traffic rows for a selected node.
pub fn seed_traffic(topology: &Topology, selected: &ResourceId) -> Result<TrafficSeed, String> {
    let node = topology
        .nodes
        .iter()
        .find(|node| &node.id == selected)
        .ok_or_else(|| format!("resource {selected} is not in the topology"))?;
    let (service, route) = match node.category {
        NodeCategory::KService => {
            let route = topology
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Owns && &edge.from == selected)
                .find_map(|edge| {
                    topology
                        .nodes
                        .iter()
                        .find(|candidate| {
                            candidate.id == edge.to && candidate.category == NodeCategory::Route
                        })
                        .map(|candidate| candidate.id.clone())
                });
            (selected.clone(), route)
        }
        NodeCategory::Route => {
            let service = topology
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Owns && edge.to == *selected)
                .find_map(|edge| {
                    topology
                        .nodes
                        .iter()
                        .find(|candidate| {
                            candidate.id == edge.from
                                && candidate.category == NodeCategory::KService
                        })
                        .map(|candidate| candidate.id.clone())
                })
                .ok_or_else(|| "the Route has no owning Knative Service".to_owned())?;
            (service, Some(selected.clone()))
        }
        _ => {
            return Err("select a Knative Service or Route to edit traffic splits".to_owned());
        }
    };
    let Some(route) = route else {
        return Ok(TrafficSeed {
            service,
            route: None,
            rows: Vec::new(),
        });
    };
    let mut rows: Vec<TrafficRow> = topology
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.from == route)
        .map(|edge| TrafficRow {
            revision: edge.to.name.clone(),
            percent: edge
                .metadata
                .get("percent")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0),
            tag: edge
                .metadata
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
        .collect();
    rows.sort_by(|left, right| left.revision.cmp(&right.revision));
    Ok(TrafficSeed {
        service,
        route: Some(route),
        rows,
    })
}

pub(super) fn validate_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Ok(());
    }
    if tag.len() > 63 {
        return Err(format!("traffic tag '{tag}' is longer than 63 characters"));
    }
    if !tag.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err(format!(
            "traffic tag '{tag}' must use lowercase letters, digits, or '-'"
        ));
    }
    if tag.starts_with('-') || tag.ends_with('-') {
        return Err(format!("traffic tag '{tag}' cannot start or end with '-'"));
    }
    Ok(())
}
