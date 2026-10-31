//! Backing operator CRD boundary.
//!
//! Task 7 owns health extraction and backing-service discovery. This module
//! freezes the extension point and re-exports relevant graph types.

pub use knect_proto::ResourceId;
pub use knect_proto::graph::{Health, NodeCategory};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;

/// Lists backing operator resources visible in the current store.
pub trait OperatorCatalog: Send + Sync {
    /// Returns backing-service identities.
    fn backing_services(&self, ctx: &CorrelationContext) -> Result<Vec<ResourceId>, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(catalog: std::sync::Arc<dyn OperatorCatalog>) -> std::sync::Arc<dyn OperatorCatalog> {
    catalog
}
