//! policy.istio correlation stage.
//!
//! Foundation stub; Task 10 replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op policy.istio correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct IstioPolicyCorrelator;

impl Correlator for IstioPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.istio"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
