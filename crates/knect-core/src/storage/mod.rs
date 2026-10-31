//! Storage tracing boundary.
//!
//! Task 8 owns PVC/PV/StorageClass/Ceph correlation and the RWO linter. This
//! module freezes the extension point and re-exports the protocol storage types.

pub use knect_proto::storage::{AccessMode, StorageConflictWarning, StorageRecord, StorageWarning};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;

/// Resolves storage records for the current store.
pub trait StorageResolver: Send + Sync {
    /// Returns storage relationships discovered in the store.
    fn records(&self, ctx: &CorrelationContext) -> Result<Vec<StorageRecord>, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(
    resolver: std::sync::Arc<dyn StorageResolver>,
) -> std::sync::Arc<dyn StorageResolver> {
    resolver
}
