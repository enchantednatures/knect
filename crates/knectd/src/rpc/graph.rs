//! `graph.*` RPC handlers.

use std::time::Duration;

use knect_proto::RpcError;
use knect_proto::error::{INTERNAL_ERROR, NOT_FOUND};
use knect_proto::graph::{NodeDetails, Topology};
use knect_proto::rpc::{GetNodeDetailsParams, GetTopologyParams, TopologySection};

use crate::app::AppState;

/// Maximum time `min_generation` may wait for catch-up.
const MIN_GENERATION_WAIT: Duration = Duration::from_secs(2);
/// Poll cadence for the bounded generation wait.
const GENERATION_POLL: Duration = Duration::from_millis(25);

/// Returns the current topology, optionally waiting briefly for a generation.
pub(crate) async fn get_topology(
    state: &AppState,
    params: &GetTopologyParams,
) -> Result<Topology, RpcError> {
    if let Some(context) = params.context.as_deref()
        && !context.is_empty()
        && context != state.context()
    {
        return Err(RpcError::new(
            NOT_FOUND,
            format!("context '{context}' is not served by this daemon"),
        ));
    }

    if let Some(min_generation) = params.min_generation {
        let deadline = tokio::time::Instant::now() + MIN_GENERATION_WAIT;
        while state.generation() < min_generation && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(GENERATION_POLL).await;
        }
    }

    let mut topology = (*state.topology()).clone();
    if let Some(include) = params.include.as_ref() {
        if !include.contains(&TopologySection::Nodes) {
            topology.nodes.clear();
        }
        if !include.contains(&TopologySection::Edges) {
            topology.edges.clear();
        }
        if !include.contains(&TopologySection::Warnings) {
            topology.warnings.clear();
        }
        if !include.contains(&TopologySection::Stats) {
            topology.stats = knect_proto::graph::TopologyStats::default();
        }
    }
    Ok(topology)
}

/// Builds node details from the topology projection and the sanitized store.
pub(crate) async fn get_node_details(
    state: &AppState,
    params: &GetNodeDetailsParams,
) -> Result<NodeDetails, RpcError> {
    let topology = state.topology();
    let node = topology
        .nodes
        .iter()
        .find(|node| node.id == params.id)
        .cloned()
        .ok_or_else(|| {
            RpcError::new(
                NOT_FOUND,
                format!("node {} is not present in the topology", params.id),
            )
        })?;

    let object = {
        let store = state.store.read().await;
        store.get(&params.id).map(|object| (*object).clone())
    };

    let yaml = if params.include_yaml.unwrap_or(false) {
        match object.as_ref() {
            Some(object) => Some(serde_yaml::to_string(object).map_err(|error| {
                RpcError::new(
                    INTERNAL_ERROR,
                    format!("cannot serialize node YAML: {error}"),
                )
            })?),
            None => None,
        }
    } else {
        None
    };

    let warnings = topology
        .warnings
        .iter()
        .filter(|warning| warning.resource.as_ref() == Some(&params.id))
        .cloned()
        .collect();

    Ok(NodeDetails {
        id: node.id,
        category: node.category,
        name: node.name,
        namespace: node.namespace,
        health: node.health,
        summary: node.summary,
        yaml,
        yaml_redacted: true,
        mutations: Vec::new(),
        policies: Vec::new(),
        links: Vec::new(),
        storage: Vec::new(),
        warnings,
        metrics: None,
        logs: Vec::new(),
    })
}
