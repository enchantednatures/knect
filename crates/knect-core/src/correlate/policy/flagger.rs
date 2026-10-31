//! policy.flagger correlation stage.
//!
//! Foundation stub; Task 10 replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op policy.flagger correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlaggerPolicyCorrelator;

impl Correlator for FlaggerPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.flagger"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
