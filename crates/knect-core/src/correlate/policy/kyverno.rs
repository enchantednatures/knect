//! policy.kyverno correlation stage.
//!
//! Foundation stub; Task 10 replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op policy.kyverno correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct KyvernoPolicyCorrelator;

impl Correlator for KyvernoPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.kyverno"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
