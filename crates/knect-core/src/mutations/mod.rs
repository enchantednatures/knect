//! Managed-fields and mutation attribution boundary.
//!
//! The module parses Kubernetes `FieldsV1` tries, maps field ownership to JSON
//! Pointer paths and rendered YAML line/column spans, and builds the three-way
//! user-intent / Knative-defaults / policy-mutators diff returned by
//! `mutations.get_diff`.

pub mod fields_v1;
pub mod managers;
mod pointer;
pub mod three_way;
pub mod yaml_index;

pub use fields_v1::{FieldsV1Error, ParsedPath, PathSegment, ResolvedPath, parse_fields_v1};
pub use managers::{ManagerClassification, classify_manager, classify_operation};
pub use three_way::{diff, records};
pub use yaml_index::{YamlIndex, YamlIndexError, render_yaml};

pub use knect_proto::ResourceId;
pub use knect_proto::mutation::{
    DiffLine, FieldOwnership, ManagerKind, MutationDiff, MutationDiffMode, MutationOperation,
    MutationRecord, MutationSource,
};

use crate::error::CoreError;

/// A mutation attribution failure.
#[derive(Debug, thiserror::Error)]
pub enum MutationError {
    /// The object has no `metadata.managedFields` entries.
    #[error("resource has no managedFields")]
    MissingManagedFields,
    /// One manager's `FieldsV1` trie is malformed.
    #[error("manager '{manager}' has invalid FieldsV1: {source}")]
    FieldsV1 {
        /// Manager whose field set failed to parse.
        manager: String,
        /// Parser diagnostic.
        #[source]
        source: FieldsV1Error,
    },
    /// The object could not be serialized as JSON.
    #[error("cannot serialize resource: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The document could not be rendered or indexed as YAML.
    #[error(transparent)]
    Yaml(#[from] YamlIndexError),
}

impl From<MutationError> for CoreError {
    fn from(error: MutationError) -> Self {
        let message = error.to_string();
        match error {
            MutationError::MissingManagedFields => Self::Store(message),
            MutationError::Serialize(source) => Self::Serialization(source),
            MutationError::FieldsV1 { .. } | MutationError::Yaml(_) => Self::Graph(message),
        }
    }
}

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
