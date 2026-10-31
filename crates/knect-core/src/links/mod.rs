//! Link discovery boundary.
//!
//! Task 6 owns the scanner implementation. This module freezes the extension
//! point and re-exports the protocol link types so downstream crates do not
//! import `knect-proto` paths directly for link payloads.

pub use knect_proto::links::{Evidence, EvidenceKind, LinkRecord, LinkVia};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// Scans sanitized objects for explicit and implicit relationships.
pub trait LinkScanner: Send + Sync {
    /// Returns discovered links; implementations may also add graph edges.
    fn scan(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<Vec<LinkRecord>, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(scanner: std::sync::Arc<dyn LinkScanner>) -> std::sync::Arc<dyn LinkScanner> {
    scanner
}
