//! eventing correlation stage.
//!
//! Foundation stub. The stage is registered at the frozen position and returns
//! success without touching the graph. The owning task replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op eventing correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct EventingCorrelator;

impl Correlator for EventingCorrelator {
    fn name(&self) -> &'static str {
        "eventing"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
