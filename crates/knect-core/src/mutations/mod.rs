//! Managed-fields and mutation attribution boundary.
//!
//! Task 9 owns the `FieldsV1` parser and three-way diff. This module freezes
//! the extension point and re-exports the protocol mutation types.

pub use knect_proto::ResourceId;
pub use knect_proto::mutation::{
    DiffLine, FieldOwnership, ManagerKind, MutationDiff, MutationDiffMode, MutationOperation,
    MutationRecord, MutationSource,
};

use crate::error::CoreError;

/// Produces a mutation attribution diff for one resource.
pub trait MutationAnalyzer: Send + Sync {
    /// Returns the diff for `id`.
    fn diff(&self, id: &ResourceId, mode: MutationDiffMode) -> Result<MutationDiff, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(
    analyzer: std::sync::Arc<dyn MutationAnalyzer>,
) -> std::sync::Arc<dyn MutationAnalyzer> {
    analyzer
}
