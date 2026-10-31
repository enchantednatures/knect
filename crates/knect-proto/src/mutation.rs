//! Field ownership and three-way mutation attribution types.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{FieldPath, ResourceId};

/// The kind of actor that wrote a managed field.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ManagerKind {
    Kubectl,
    Controller,
    Knative,
    Istio,
    Flagger,
    Kyverno,
    Helm,
    Operator,
    #[serde(other)]
    Other,
}

/// The managed-fields operation recorded for a field set.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperation {
    Apply,
    Update,
    Create,
    Delete,
    #[serde(other)]
    Other,
}

/// A field claimed by one manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldOwnership {
    /// JSON Pointer to the field, for example `/spec/template/spec/containers/0/image`.
    pub path: FieldPath,
    /// One-based line in the rendered YAML, when the field is present.
    pub yaml_line: Option<u32>,
    /// One-based column in the rendered YAML, when the field is present.
    pub yaml_column: Option<u32>,
    /// Redacted preview of the field value. Never contains secret material.
    pub value_preview: Option<String>,
}

/// One manager's contribution to an object's managed fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MutationRecord {
    /// Manager name from `metadata.managedFields`.
    pub manager: String,
    /// Manager category used by the UI.
    pub manager_kind: ManagerKind,
    /// Operation recorded for the field set.
    pub operation: MutationOperation,
    /// Subresource, for example `status`, when the entry targets one.
    pub subresource: Option<String>,
    /// Time recorded by the API server for this field set.
    #[schemars(with = "Option<String>")]
    pub time: Option<DateTime<Utc>>,
    /// Fields owned by this entry.
    #[serde(default)]
    pub paths: Vec<FieldOwnership>,
}

/// Which of the three diff panes a line belongs to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MutationSource {
    /// What the user asked for.
    UserIntent,
    /// Defaults injected by Knative or another controller.
    KnativeDefaults,
    /// Mutations applied by policy or service-mesh engines.
    PolicyMutators,
    #[serde(other)]
    Unknown,
}

/// Diff rendering mode requested by `mutations.get_diff`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MutationDiffMode {
    /// Compare user intent, Knative defaults, and policy mutators.
    #[default]
    ThreeWay,
    /// Group ownership by manager only.
    Managers,
}

/// One rendered line of a mutation diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DiffLine {
    /// One-based line number in the rendered diff.
    pub line_number: u32,
    /// Pane the line belongs to.
    pub side: MutationSource,
    /// Manager or policy that owns the field on this line.
    pub owner: Option<String>,
    /// Line contents with secret values redacted.
    pub text: String,
    /// True when this line differs from the adjacent panes.
    #[serde(default)]
    pub changed: bool,
    /// Field path represented by the line, when known.
    pub path: Option<FieldPath>,
}

/// Three-way diff for one resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MutationDiff {
    /// Resource being attributed.
    pub resource: ResourceId,
    /// Rendered user-intent document, when available.
    pub user_intent: Option<String>,
    /// Rendered document with Knative defaults applied.
    pub knative_defaults: Option<String>,
    /// Rendered document with policy and mesh mutations applied.
    pub policy_mutators: Option<String>,
    /// Line-level ownership and diff content.
    #[serde(default)]
    pub lines: Vec<DiffLine>,
}
