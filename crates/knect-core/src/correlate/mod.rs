//! Correlation registry.
//!
//! Correlators are pure functions over [`CorrelationContext`] plus a
//! [`GraphBuilder`]. The pipeline order is frozen at the foundation milestone:
//! `serving -> eventing -> links -> operators -> storage -> policy -> mutations`.
//! Every stage is present as a no-op stub so later tasks can fill bodies
//! without editing this file.

pub mod eventing;
pub mod links;
pub mod mutations;
pub mod operators;
pub mod policy;
pub mod serving;
pub mod storage;

use std::sync::Arc;

use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::secret::SecretMaterialRegistry;
use crate::store::ResourceStore;

/// Inputs shared by every correlator in one recompute generation.
#[derive(Debug, Clone)]
pub struct CorrelationContext {
    /// Kubernetes context name.
    pub context: String,
    /// Graph generation being produced.
    pub generation: u64,
    /// Sanitized resource store snapshot.
    pub store: Arc<ResourceStore>,
    /// Secret material observed by watchers, keyed by resource identity.
    pub secret_material: Arc<SecretMaterialRegistry>,
}

impl CorrelationContext {
    /// Creates a correlation context.
    #[must_use]
    pub fn new(context: impl Into<String>, generation: u64, store: Arc<ResourceStore>) -> Self {
        Self::new_with_secrets(
            context,
            generation,
            store,
            Arc::new(SecretMaterialRegistry::new()),
        )
    }

    /// Creates a correlation context with an explicit Secret registry.
    #[must_use]
    pub fn new_with_secrets(
        context: impl Into<String>,
        generation: u64,
        store: Arc<ResourceStore>,
        secret_material: Arc<SecretMaterialRegistry>,
    ) -> Self {
        Self {
            context: context.into(),
            generation,
            store,
            secret_material,
        }
    }

    /// Replaces the Secret registry.
    #[must_use]
    pub fn with_secret_material(mut self, registry: Arc<SecretMaterialRegistry>) -> Self {
        self.secret_material = registry;
        self
    }
}

/// A pure graph correlation phase.
pub trait Correlator: Send + Sync {
    /// Stable stage name. This is part of the frozen registry contract.
    fn name(&self) -> &'static str;

    /// Adds nodes and edges to the graph.
    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError>;
}

/// Frozen correlation order.
pub const STAGES: &[&str] = &[
    "serving",
    "eventing",
    "links",
    "operators",
    "storage",
    "policy",
    "mutations",
];

/// Ordered correlation registry.
pub struct CorrelationPipeline {
    correlators: Vec<Arc<dyn Correlator>>,
}

impl CorrelationPipeline {
    /// Builds the default registry with every later stage pre-wired.
    #[must_use]
    pub fn new() -> Self {
        Self {
            correlators: vec![
                Arc::new(serving::ServingCorrelator),
                Arc::new(eventing::EventingCorrelator),
                Arc::new(links::LinksCorrelator),
                Arc::new(operators::OperatorsCorrelator),
                Arc::new(storage::StorageCorrelator),
                Arc::new(policy::PolicyCorrelator::new()),
                Arc::new(mutations::MutationsCorrelator),
            ],
        }
    }

    /// Builds a registry from a caller-supplied list, requiring the frozen order.
    pub fn from_correlators(correlators: Vec<Arc<dyn Correlator>>) -> Result<Self, CoreError> {
        let names: Vec<&'static str> = correlators.iter().map(|c| c.name()).collect();
        if names != STAGES {
            return Err(CoreError::Graph(format!(
                "correlator registry must be ordered {STAGES:?}, got {names:?}"
            )));
        }
        Ok(Self { correlators })
    }

    /// Runs every correlator in registry order.
    pub fn run(&self, ctx: &CorrelationContext, graph: &mut GraphBuilder) -> Result<(), CoreError> {
        for correlator in &self.correlators {
            correlator.correlate(ctx, graph)?;
        }
        Ok(())
    }

    /// Returns stage names in execution order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.correlators.iter().map(|c| c.name()).collect()
    }

    /// Number of registered stages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.correlators.len()
    }

    /// Always false: the registry is pre-populated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.correlators.is_empty()
    }
}

impl Default for CorrelationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CorrelationPipeline {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CorrelationPipeline")
            .field("stages", &self.names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::graph::TopologyStats;

    #[test]
    fn registry_order_is_frozen() {
        let pipeline = CorrelationPipeline::new();
        assert_eq!(pipeline.names(), STAGES.to_vec());
        assert_eq!(pipeline.len(), 7);
        assert!(!pipeline.is_empty());
    }

    #[test]
    fn pipeline_runs_noop_stubs_and_leaves_empty_topology() {
        let store = Arc::new(ResourceStore::new());
        let ctx = CorrelationContext::new("fixture", 0, store);
        let mut graph = GraphBuilder::new("fixture");
        CorrelationPipeline::new()
            .run(&ctx, &mut graph)
            .expect("stubs succeed");
        let topology = graph.build();
        assert!(topology.nodes.is_empty());
        assert!(topology.edges.is_empty());
        assert_eq!(topology.stats, TopologyStats::default());
    }

    #[test]
    fn wrong_registry_order_is_rejected() {
        let correlators: Vec<Arc<dyn Correlator>> = vec![
            Arc::new(eventing::EventingCorrelator),
            Arc::new(serving::ServingCorrelator),
        ];
        let error = CorrelationPipeline::from_correlators(correlators)
            .expect_err("wrong order must be rejected");
        match error {
            CoreError::Graph(message) => assert!(message.contains("serving")),
            other => panic!("unexpected error: {other}"),
        }
    }
}
