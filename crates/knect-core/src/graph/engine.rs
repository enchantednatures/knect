//! Graph assembly and the stable `petgraph` engine.
//!
//! [`GraphBuilder`] is the mutable assembly surface correlators receive.
//! [`GraphEngine`] owns the stable `petgraph` representation and projects a
//! deterministic [`Topology`]. External identity is always [`ResourceId`];
//! `NodeIndex` values never leave this module except through explicit
//! accessors used by graph-specific tasks.

use std::collections::BTreeMap;

use knect_proto::graph::{Edge, Topology, Warning};
use knect_proto::{ResourceId, ResourceNode, SCHEMA_VERSION};
use petgraph::stable_graph::{EdgeIndex, NodeIndex, StableDiGraph};

use crate::error::CoreError;
use crate::graph::stats::stats_for_truncated;

/// Mutable graph assembly surface passed to correlators.
#[derive(Debug, Clone)]
pub struct GraphBuilder {
    context: String,
    generation: u64,
    truncated: bool,
    nodes: BTreeMap<ResourceId, ResourceNode>,
    edges: BTreeMap<String, Edge>,
    warnings: Vec<Warning>,
}

impl GraphBuilder {
    /// Creates an empty builder for a Kubernetes context.
    #[must_use]
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            generation: 0,
            truncated: false,
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Sets the generation recorded in the topology projection.
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// Sets the truncation flag recorded in the topology statistics.
    #[must_use]
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Changes the context name.
    pub fn set_context(&mut self, context: impl Into<String>) {
        self.context = context.into();
    }

    /// Returns the context name.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Sets the generation.
    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Returns the generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Sets the truncation flag.
    pub fn set_truncated(&mut self, truncated: bool) {
        self.truncated = truncated;
    }

    /// Returns the truncation flag.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Adds a node. Returns `false` when the identity already exists.
    pub fn add_node(&mut self, node: ResourceNode) -> bool {
        if self.nodes.contains_key(&node.id) {
            return false;
        }
        self.nodes.insert(node.id.clone(), node);
        true
    }

    /// Inserts or replaces a node, returning the previous value.
    pub fn upsert_node(&mut self, node: ResourceNode) -> Option<ResourceNode> {
        self.nodes.insert(node.id.clone(), node)
    }

    /// Removes a node and every incident edge.
    pub fn remove_node(&mut self, id: &ResourceId) -> Option<ResourceNode> {
        let node = self.nodes.remove(id)?;
        self.edges
            .retain(|_, edge| &edge.from != id && &edge.to != id);
        Some(node)
    }

    /// Adds an edge. Returns `false` when the edge ID already exists.
    ///
    /// Endpoint existence is validated by [`GraphEngine::apply`]; correlators
    /// may add edges before the corresponding nodes in later phases.
    pub fn add_edge(&mut self, edge: Edge) -> bool {
        if self.edges.contains_key(&edge.id) {
            return false;
        }
        self.edges.insert(edge.id.clone(), edge);
        true
    }

    /// Inserts or replaces an edge, returning the previous value.
    pub fn upsert_edge(&mut self, edge: Edge) -> Option<Edge> {
        self.edges.insert(edge.id.clone(), edge)
    }

    /// Removes an edge by ID.
    pub fn remove_edge(&mut self, id: &str) -> Option<Edge> {
        self.edges.remove(id)
    }

    /// Returns a node by identity.
    #[must_use]
    pub fn node(&self, id: &ResourceId) -> Option<&ResourceNode> {
        self.nodes.get(id)
    }

    /// Returns an edge by ID.
    #[must_use]
    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.get(id)
    }

    /// Returns true when the node exists.
    #[must_use]
    pub fn contains_node(&self, id: &ResourceId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// True when the builder has no nodes and no edges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    /// Adds a topology warning.
    pub fn add_warning(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Returns the recorded warnings.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Projects the builder into a deterministic topology.
    #[must_use]
    pub fn build(self) -> Topology {
        let Self {
            context,
            generation,
            truncated,
            nodes,
            edges,
            warnings,
        } = self;
        let nodes: Vec<ResourceNode> = nodes.into_values().collect();
        let edges: Vec<Edge> = edges.into_values().collect();
        let stats = stats_for_truncated(&nodes, &edges, &warnings, truncated);
        Topology {
            schema_version: SCHEMA_VERSION,
            context,
            generation,
            nodes,
            edges,
            warnings,
            stats,
        }
    }

    /// Projects without consuming the builder.
    #[must_use]
    pub fn to_topology(&self) -> Topology {
        let nodes: Vec<ResourceNode> = self.nodes.values().cloned().collect();
        let edges: Vec<Edge> = self.edges.values().cloned().collect();
        let stats = stats_for_truncated(&nodes, &edges, &self.warnings, self.truncated);
        Topology {
            schema_version: SCHEMA_VERSION,
            context: self.context.clone(),
            generation: self.generation,
            nodes,
            edges,
            warnings: self.warnings.clone(),
            stats,
        }
    }
}

/// Stable graph engine backed by `petgraph::stable_graph::StableDiGraph`.
#[derive(Debug)]
pub struct GraphEngine {
    graph: StableDiGraph<ResourceNode, Edge>,
    node_ids: BTreeMap<ResourceId, NodeIndex>,
    edge_ids: BTreeMap<String, EdgeIndex>,
    context: String,
    generation: u64,
    truncated: bool,
    warnings: Vec<Warning>,
}

impl GraphEngine {
    /// Creates an empty engine for a Kubernetes context.
    #[must_use]
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            graph: StableDiGraph::new(),
            node_ids: BTreeMap::new(),
            edge_ids: BTreeMap::new(),
            context: context.into(),
            generation: 0,
            truncated: false,
            warnings: Vec::new(),
        }
    }

    /// Replaces the graph contents with a builder snapshot.
    ///
    /// Retained `ResourceId` values keep their stable `NodeIndex` and retained
    /// edge IDs keep their stable `EdgeIndex`. Edges whose endpoints are not
    /// present are dropped with an `edge_missing_endpoint` warning.
    pub fn apply(&mut self, builder: GraphBuilder) -> Result<(), CoreError> {
        let GraphBuilder {
            context,
            generation,
            truncated,
            nodes,
            edges,
            warnings,
        } = builder;
        if !context.is_empty() {
            self.context = context;
        }
        self.generation = generation;
        self.truncated = truncated;
        self.warnings = warnings;

        self.reconcile_edges(&edges);
        self.reconcile_nodes(&nodes);
        self.upsert_edges(edges);
        Ok(())
    }

    fn reconcile_edges(&mut self, desired: &BTreeMap<String, Edge>) {
        let stale: Vec<String> = self
            .edge_ids
            .keys()
            .filter(|id| !desired.contains_key(*id))
            .cloned()
            .collect();
        for id in stale {
            if let Some(index) = self.edge_ids.remove(&id) {
                self.graph.remove_edge(index);
            }
        }
    }

    fn reconcile_nodes(&mut self, desired: &BTreeMap<ResourceId, ResourceNode>) {
        let stale: Vec<ResourceId> = self
            .node_ids
            .keys()
            .filter(|id| !desired.contains_key(*id))
            .cloned()
            .collect();
        for id in stale {
            if let Some(index) = self.node_ids.remove(&id) {
                self.remove_incident_edges(index);
                self.graph.remove_node(index);
            }
        }

        for (id, node) in desired {
            match self.node_ids.get(id).copied() {
                Some(index) => {
                    self.graph[index] = node.clone();
                }
                None => {
                    let index = self.graph.add_node(node.clone());
                    self.node_ids.insert(id.clone(), index);
                }
            }
        }
    }

    fn remove_incident_edges(&mut self, node: NodeIndex) {
        let incident: Vec<EdgeIndex> = self
            .graph
            .edge_indices()
            .filter(|index| {
                self.graph
                    .edge_endpoints(*index)
                    .is_some_and(|(from, to)| from == node || to == node)
            })
            .collect();
        for index in incident {
            if let Some(weight) = self.graph.edge_weight(index) {
                self.edge_ids.remove(&weight.id);
            }
            self.graph.remove_edge(index);
        }
    }

    fn upsert_edges(&mut self, edges: BTreeMap<String, Edge>) {
        for (id, edge) in edges {
            let from = self.node_ids.get(&edge.from).copied();
            let to = self.node_ids.get(&edge.to).copied();
            let (Some(from), Some(to)) = (from, to) else {
                if let Some(index) = self.edge_ids.remove(&id) {
                    self.graph.remove_edge(index);
                }
                self.warnings.push(Warning {
                    code: "edge_missing_endpoint".to_owned(),
                    message: format!("edge {id} references a node that is not in the topology"),
                    severity: knect_proto::graph::Severity::Medium,
                    resource: None,
                });
                continue;
            };

            match self.edge_ids.get(&id).copied() {
                Some(index) if self.graph.edge_endpoints(index) == Some((from, to)) => {
                    self.graph[index] = edge;
                }
                Some(index) => {
                    self.graph.remove_edge(index);
                    let replacement = self.graph.add_edge(from, to, edge);
                    self.edge_ids.insert(id, replacement);
                }
                None => {
                    let index = self.graph.add_edge(from, to, edge);
                    self.edge_ids.insert(id, index);
                }
            }
        }
    }

    /// Returns a node by identity.
    #[must_use]
    pub fn node(&self, id: &ResourceId) -> Option<&ResourceNode> {
        self.node_index(id)
            .and_then(|index| self.graph.node_weight(index))
    }

    /// Returns an edge by edge ID.
    #[must_use]
    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edge_index(id)
            .and_then(|index| self.graph.edge_weight(index))
    }

    /// Returns true when the node exists.
    #[must_use]
    pub fn contains_node(&self, id: &ResourceId) -> bool {
        self.node_ids.contains_key(id)
    }

    /// Number of live nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }

    /// Number of live edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_ids.len()
    }

    /// Returns the current generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the context name.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Returns the stable external-to-internal node mapping.
    #[must_use]
    pub fn node_index(&self, id: &ResourceId) -> Option<NodeIndex> {
        self.node_ids.get(id).copied()
    }

    /// Returns the external-to-internal edge mapping.
    #[must_use]
    pub fn edge_index(&self, id: &str) -> Option<EdgeIndex> {
        self.edge_ids.get(id).copied()
    }

    /// Returns the immutable underlying graph for graph-specific tasks.
    #[must_use]
    pub fn graph(&self) -> &StableDiGraph<ResourceNode, Edge> {
        &self.graph
    }

    /// Number of warnings recorded by the last application.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// Projects a deterministic topology.
    #[must_use]
    pub fn topology(&self) -> Topology {
        let mut nodes: Vec<ResourceNode> = self.graph.node_weights().cloned().collect();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        let mut edges: Vec<Edge> = self.graph.edge_weights().cloned().collect();
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        let stats = stats_for_truncated(&nodes, &edges, &self.warnings, self.truncated);
        Topology {
            schema_version: SCHEMA_VERSION,
            context: self.context.clone(),
            generation: self.generation,
            nodes,
            edges,
            warnings: self.warnings.clone(),
            stats,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use knect_proto::NodeCategory;
    use knect_proto::graph::EdgeKind;
    use knect_proto::graph::{Health, Severity, TopologyStats};

    fn timestamp() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    fn node(kind: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", kind, "default", name),
            category: NodeCategory::KService,
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: format!("{kind} {name}"),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: timestamp(),
            last_seen: timestamp(),
        }
    }

    fn edge(id: &str, from: &ResourceNode, to: &ResourceNode) -> Edge {
        Edge {
            id: id.to_owned(),
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn node_and_edge_lifecycle() {
        let mut builder = GraphBuilder::new("test").with_generation(7);
        let service = node("Service", "hello");
        let revision = node("Revision", "hello-00001");

        assert!(builder.add_node(service.clone()));
        assert!(!builder.add_node(service.clone()));
        assert!(builder.add_node(revision.clone()));
        assert_eq!(builder.node_count(), 2);

        let owns = edge("service-owns-revision", &service, &revision);
        assert!(builder.add_edge(owns.clone()));
        assert!(!builder.add_edge(owns.clone()));
        assert_eq!(builder.edge_count(), 1);

        let replacement = node("Service", "hello");
        assert!(builder.upsert_node(replacement).is_some());
        assert!(builder.remove_edge("service-owns-revision").is_some());
        assert!(builder.remove_edge("service-owns-revision").is_none());

        assert!(builder.remove_node(&revision.id).is_some());
        assert_eq!(builder.node_count(), 1);
        assert!(builder.remove_node(&revision.id).is_none());
        assert!(!builder.contains_node(&revision.id));

        let topology = builder.build();
        assert_eq!(topology.schema_version, SCHEMA_VERSION);
        assert_eq!(topology.context, "test");
        assert_eq!(topology.generation, 7);
        assert_eq!(topology.stats.node_count, 1);
        assert_eq!(topology.stats.edge_count, 0);
    }

    #[test]
    fn removing_a_node_removes_incident_edges() {
        let mut builder = GraphBuilder::new("test");
        let a = node("Service", "a");
        let b = node("Service", "b");
        builder.add_node(a.clone());
        builder.add_node(b.clone());
        builder.add_edge(edge("a-b", &a, &b));
        builder.remove_node(&a.id);
        assert_eq!(builder.edge_count(), 0);
    }

    #[test]
    fn duplicate_edge_ids_are_rejected_and_upsert_replaces() {
        let mut builder = GraphBuilder::new("test");
        let a = node("Service", "a");
        let b = node("Service", "b");
        builder.add_node(a.clone());
        builder.add_node(b.clone());
        let original = edge("e1", &a, &b);
        builder.add_edge(original.clone());
        let mut replacement = original.clone();
        replacement.label = Some("replaced".to_owned());
        assert!(builder.upsert_edge(replacement).is_some());
        assert_eq!(
            builder.edge("e1").and_then(|edge| edge.label.as_deref()),
            Some("replaced")
        );
    }

    #[test]
    fn topology_is_deterministic_and_categories_are_sorted() {
        let mut builder = GraphBuilder::new("test");
        let zeta = node("Service", "zeta");
        let alpha = node("Service", "alpha");
        let mut pod = node("Pod", "web");
        pod.category = NodeCategory::Pod;
        builder.add_node(zeta.clone());
        builder.add_node(alpha.clone());
        builder.add_node(pod.clone());
        builder.add_edge(edge("z-edge", &zeta, &alpha));
        builder.add_edge(edge("a-edge", &alpha, &pod));

        let topology = builder.build();
        let names: Vec<&str> = topology
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(names, vec!["web", "alpha", "zeta"]);
        let edge_ids: Vec<&str> = topology.edges.iter().map(|edge| edge.id.as_str()).collect();
        assert_eq!(edge_ids, vec!["a-edge", "z-edge"]);
        let categories: Vec<&str> = topology
            .stats
            .categories
            .iter()
            .map(|count| crate::graph::stats::category_key(count.category))
            .collect();
        assert_eq!(categories, vec!["k_service", "pod"]);
    }

    #[test]
    fn empty_topology_stats_are_zero() {
        let topology = GraphBuilder::new("fixture").build();
        assert_eq!(topology.context, "fixture");
        assert!(topology.nodes.is_empty());
        assert!(topology.edges.is_empty());
        assert_eq!(topology.stats, TopologyStats::default());
    }

    #[test]
    fn truncation_flag_reaches_stats() {
        let topology = GraphBuilder::new("fixture").with_truncated(true).build();
        assert!(topology.stats.truncated);

        let mut engine = GraphEngine::new("fixture");
        engine
            .apply(GraphBuilder::new("fixture").with_truncated(true))
            .expect("apply succeeds");
        assert!(engine.topology().stats.truncated);
    }

    #[test]
    fn engine_keeps_stable_node_indices_and_drops_invalid_edges() {
        let mut builder = GraphBuilder::new("test").with_generation(1);
        let service = node("Service", "hello");
        let revision = node("Revision", "hello-00001");
        builder.add_node(service.clone());
        builder.add_node(revision.clone());
        builder.add_edge(edge("owns", &service, &revision));

        let mut engine = GraphEngine::new("test");
        engine.apply(builder).expect("apply succeeds");
        let service_index = engine.node_index(&service.id).expect("service present");
        assert_eq!(engine.node_count(), 2);
        assert_eq!(engine.edge_count(), 1);

        let mut next = GraphBuilder::new("test").with_generation(2);
        next.add_node(service.clone());
        let ghost = node("Service", "ghost");
        next.add_edge(edge("ghost-edge", &service, &ghost));
        engine.apply(next).expect("apply succeeds");
        assert_eq!(engine.node_index(&service.id), Some(service_index));
        assert_eq!(engine.node_count(), 1);
        assert_eq!(engine.edge_count(), 0);
        assert_eq!(engine.topology().warnings.len(), 1);
        assert!(engine.edge("ghost-edge").is_none());
    }

    #[test]
    fn engine_keeps_stable_edge_indices_across_updates() {
        let mut builder = GraphBuilder::new("test").with_generation(1);
        let service = node("Service", "hello");
        let revision = node("Revision", "hello-00001");
        builder.add_node(service.clone());
        builder.add_node(revision.clone());
        builder.add_edge(edge("owns", &service, &revision));

        let mut engine = GraphEngine::new("test");
        engine.apply(builder).expect("apply succeeds");
        let edge_index = engine.edge_index("owns").expect("edge present");

        let mut next = GraphBuilder::new("test").with_generation(2);
        next.add_node(service);
        next.add_node(revision.clone());
        let mut replacement = edge("owns", &node("Service", "hello"), &revision);
        replacement.label = Some("still owned".to_owned());
        next.add_edge(replacement);
        engine.apply(next).expect("apply succeeds");

        assert_eq!(engine.edge_index("owns"), Some(edge_index));
        assert_eq!(
            engine.edge("owns").and_then(|edge| edge.label.as_deref()),
            Some("still owned")
        );
        assert_eq!(engine.edge_count(), 1);
    }

    #[test]
    fn edge_replacement_does_not_leave_stale_edges() {
        let mut builder = GraphBuilder::new("test");
        let a = node("Service", "a");
        let b = node("Service", "b");
        let c = node("Service", "c");
        builder.add_node(a.clone());
        builder.add_node(b.clone());
        builder.add_node(c.clone());
        builder.add_edge(edge("same-id", &a, &b));

        let mut engine = GraphEngine::new("test");
        engine.apply(builder).expect("apply succeeds");

        let mut next = GraphBuilder::new("test");
        next.add_node(a.clone());
        next.add_node(b.clone());
        next.add_node(c.clone());
        next.add_edge(edge("same-id", &a, &c));
        engine.apply(next).expect("apply succeeds");

        let updated = engine.edge("same-id").expect("edge present");
        assert_eq!(updated.to, c.id);
        assert_eq!(engine.edge_count(), 1);
        assert!(engine.edge_index("replaced").is_none());
    }

    #[test]
    fn node_removal_drops_incident_edges_from_the_engine_index() {
        let mut builder = GraphBuilder::new("test");
        let a = node("Service", "a");
        let b = node("Service", "b");
        builder.add_node(a.clone());
        builder.add_node(b.clone());
        builder.add_edge(edge("a-b", &a, &b));

        let mut engine = GraphEngine::new("test");
        engine.apply(builder).expect("apply succeeds");
        assert_eq!(engine.edge_count(), 1);

        let mut next = GraphBuilder::new("test");
        next.add_node(a);
        engine.apply(next).expect("apply succeeds");
        assert_eq!(engine.node_count(), 1);
        assert_eq!(engine.edge_count(), 0);
        assert!(engine.edge_index("a-b").is_none());
    }

    #[test]
    fn engine_projection_is_deterministic() {
        let mut builder = GraphBuilder::new("test").with_generation(3);
        let b = node("Service", "b");
        let a = node("Service", "a");
        builder.add_node(b);
        builder.add_node(a);
        builder.add_warning(Warning {
            code: "test".to_owned(),
            message: "warning".to_owned(),
            severity: Severity::Info,
            resource: None,
        });
        let mut engine = GraphEngine::new("test");
        engine.apply(builder).expect("apply succeeds");
        let topology = engine.topology();
        assert_eq!(topology.generation, 3);
        assert_eq!(topology.nodes[0].name, "a");
        assert_eq!(topology.nodes[1].name, "b");
        assert_eq!(topology.stats.warning_count, 1);
        assert_eq!(engine.warning_count(), 1);
    }
}
