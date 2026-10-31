//! links correlation stage.
//!
//! Foundation stub. The stage is registered at the frozen position and returns
//! success without touching the graph. The owning task replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op links correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinksCorrelator;

impl Correlator for LinksCorrelator {
    fn name(&self) -> &'static str {
        "links"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
