//! `knect graph` focused topology inspection.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use clap::Args;
use knect_proto::Topology;
use knect_proto::graph::{Edge, ResourceNode};

use super::target;
use super::{CliError, json_text};

/// Arguments for `knect graph`.
#[derive(Debug, Clone, Args)]
pub struct GraphArgs {
    /// Emit JSON instead of a terminal view.
    #[arg(long)]
    pub json: bool,
    /// Resource as `kind/name` or `apiVersion/kind/namespace/name`.
    pub resource: Option<String>,
    /// Namespace for the resource.
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
}

/// Runs the graph subcommand.
pub async fn run(
    args: GraphArgs,
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<(), CliError> {
    let connected = target::connect(socket, daemon_url).await?;
    let topology = target::fetch_topology(Arc::clone(&connected.client)).await?;
    let Some(resource) = args.resource.as_deref() else {
        print_topology(&topology, args.json)?;
        return Ok(());
    };
    let parsed = target::parse_target(resource)?;
    let id = target::resolve_target(&topology, &parsed, args.namespace.as_deref())?;
    let node = topology
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or_else(|| CliError::NotFound(id.to_string()))?;
    let incoming = incident(&topology, node, true);
    let outgoing = incident(&topology, node, false);
    if args.json {
        let focused = serde_json::json!({
            "context": topology.context,
            "generation": topology.generation,
            "resource": node,
            "incoming": incoming,
            "outgoing": outgoing,
        });
        println!("{}", json_text(&focused)?);
    } else {
        print_focused(&topology, node, &incoming, &outgoing);
    }
    Ok(())
}

fn incident<'a>(topology: &'a Topology, node: &ResourceNode, incoming: bool) -> Vec<&'a Edge> {
    let mut edges: Vec<&Edge> = topology
        .edges
        .iter()
        .filter(|edge| {
            if incoming {
                edge.to == node.id
            } else {
                edge.from == node.id
            }
        })
        .collect();
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges
}

fn print_topology(topology: &Topology, json: bool) -> Result<(), CliError> {
    if json {
        println!("{}", json_text(topology)?);
        return Ok(());
    }
    println!(
        "context: {}  generation: {}",
        topology.context, topology.generation
    );
    println!(
        "nodes: {}  edges: {}  warnings: {}",
        topology.nodes.len(),
        topology.edges.len(),
        topology.warnings.len()
    );
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &topology.nodes {
        *counts
            .entry(target::category_label(node.category))
            .or_default() += 1;
    }
    for (category, count) in counts {
        println!("{category:<24} {count}");
    }
    Ok(())
}

fn print_focused(topology: &Topology, node: &ResourceNode, incoming: &[&Edge], outgoing: &[&Edge]) {
    println!(
        "context: {}  generation: {}",
        topology.context, topology.generation
    );
    println!(
        "resource: {} {}/{}  [{}]",
        node.id.kind,
        node.namespace.as_deref().unwrap_or("cluster-scoped"),
        node.name,
        health_label(node.health)
    );
    if !node.summary.is_empty() {
        println!("summary: {}", node.summary);
    }
    println!("outgoing ({}):", outgoing.len());
    for edge in outgoing {
        println!(
            "  -> {} {}/{}  [{}]{}",
            edge.to.kind,
            edge.to.namespace.as_deref().unwrap_or("cluster-scoped"),
            edge.to.name,
            edge_kind_label(edge),
            edge_label(edge)
        );
    }
    println!("incoming ({}):", incoming.len());
    for edge in incoming {
        println!(
            "  <- {} {}/{}  [{}]{}",
            edge.from.kind,
            edge.from.namespace.as_deref().unwrap_or("cluster-scoped"),
            edge.from.name,
            edge_kind_label(edge),
            edge_label(edge)
        );
    }
}

fn edge_kind_label(edge: &Edge) -> String {
    format!("{:?}", edge.kind)
}

fn edge_label(edge: &Edge) -> String {
    edge.label
        .as_deref()
        .map(|label| format!(" {label}"))
        .unwrap_or_default()
}

fn health_label(health: knect_proto::graph::Health) -> &'static str {
    match health {
        knect_proto::graph::Health::Healthy => "healthy",
        knect_proto::graph::Health::Degraded => "degraded",
        knect_proto::graph::Health::Progressing => "progressing",
        knect_proto::graph::Health::Blocked => "blocked",
        knect_proto::graph::Health::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::ResourceId;
    use knect_proto::graph::{EdgeKind, Health, NodeCategory, TopologyStats};
    use std::collections::BTreeMap;

    #[test]
    fn incident_edges_are_sorted_by_id() {
        let node = ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", "Service", "default", "hello"),
            category: NodeCategory::KService,
            name: "hello".to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        let edge = |id: &str| Edge {
            id: id.to_owned(),
            from: node.id.clone(),
            to: node.id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let topology = Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            nodes: vec![node.clone()],
            edges: vec![edge("b"), edge("a")],
            warnings: Vec::new(),
            stats: TopologyStats::default(),
        };
        let ids: Vec<&str> = incident(&topology, &node, false)
            .iter()
            .map(|edge| edge.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
