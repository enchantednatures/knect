//! storage correlation stage.
//!
//! Foundation stub. The stage is registered at the frozen position and returns
//! success without touching the graph. The owning task replaces the body.

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// No-op storage correlator placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct StorageCorrelator;

impl Correlator for StorageCorrelator {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn correlate(
        &self,
        _ctx: &CorrelationContext,
        _graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}
