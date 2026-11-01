//! Policy correlation stage.
//!
//! The stage aggregates the Istio, Flagger, and Kyverno stubs in a fixed order.
//! Task 10 replaces each sub-correlator body; the aggregation order is frozen.

pub mod flagger;
pub mod istio;
pub mod kyverno;

mod redact;
mod resolve;
mod support;

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// Aggregates Istio, Flagger, and Kyverno policy correlation.
#[derive(Debug, Default, Clone, Copy)]
pub struct PolicyCorrelator {
    /// Istio policy correlation stub.
    pub istio: istio::IstioPolicyCorrelator,
    /// Flagger canary correlation stub.
    pub flagger: flagger::FlaggerPolicyCorrelator,
    /// Kyverno report correlation stub.
    pub kyverno: kyverno::KyvernoPolicyCorrelator,
}

impl PolicyCorrelator {
    /// Creates the policy aggregator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Correlator for PolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        self.istio.correlate(ctx, graph)?;
        self.flagger.correlate(ctx, graph)?;
        self.kyverno.correlate(ctx, graph)
    }
}
