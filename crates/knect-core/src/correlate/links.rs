//! Links correlation stage.
//!
//! Runs the production [`crate::links::DiscoveryLinkScanner`] over the
//! sanitized store and projects the discovered candidates into deterministic
//! graph edges. Explicit binding and heuristic connection evidence is attached
//! to each edge with secret and ConfigMap values redacted.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::links::{DiscoveryLinkScanner, LinkScanner};

/// Correlates explicit bindings and heuristic connections into the graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinksCorrelator;

impl Correlator for LinksCorrelator {
    fn name(&self) -> &'static str {
        "links"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let scanner = DiscoveryLinkScanner;
        let _records = scanner.scan(ctx, graph)?;
        Ok(())
    }
}
