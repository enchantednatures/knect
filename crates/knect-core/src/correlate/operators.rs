//! Backing operator CRD correlation.
//!
//! Projects every recognized operator CRD into a `BackingService` or `Topic`
//! node and connects existing Brokers, Topics, Services, and Endpoints to their
//! backing CRDs. Missing CRDs produce neither a node nor an edge.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{NodeSpec, resource_node, warning};
use crate::operators::{self, OperatorFamily};

/// Correlates backing operator CRDs into the graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct OperatorsCorrelator;

impl Correlator for OperatorsCorrelator {
    fn name(&self) -> &'static str {
        "operators"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let nodes = operators::describe_all(&ctx.store);
        for operator in &nodes {
            let Some(object) = ctx.store.get(&operator.id) else {
                continue;
            };
            graph.add_node(resource_node(
                &object,
                NodeSpec {
                    category: operator.category,
                    health: operator.health,
                    summary: operator.summary.clone(),
                    badges: operator.badges.clone(),
                },
            ));
        }

        add_malformed_warnings(ctx, graph);
        operators::add_edges(&ctx.store, graph, &nodes);
        Ok(())
    }
}

fn add_malformed_warnings(ctx: &CorrelationContext, graph: &mut GraphBuilder) {
    for (id, object) in ctx.store.iter() {
        if OperatorFamily::from_id(id).is_none() {
            continue;
        }
        if object.metadata.name.as_deref().is_none_or(str::is_empty) {
            graph.add_warning(warning(
                "malformed_object",
                format!("{id} has no metadata.name"),
                Some(id.clone()),
            ));
        }
    }
}
