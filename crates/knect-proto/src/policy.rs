//! Policy, validation, and compliance result types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::Warning;
use crate::ids::ResourceId;

pub use crate::graph::Severity;

/// Policy engine that produced a result.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    Istio,
    Kyverno,
    Flagger,
    #[serde(other)]
    Other,
}

/// Outcome of evaluating one policy rule.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PolicyResult {
    Pass,
    Fail,
    Warn,
    Error,
    Skip,
}

/// A single policy finding attached to a resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyRecord {
    /// Stable policy result ID within a generation.
    pub id: String,
    /// Policy name.
    pub name: String,
    /// Policy namespace, or `None` for cluster-scoped policies.
    pub namespace: Option<String>,
    /// Engine that produced the result.
    pub kind: PolicyKind,
    /// Resource the finding applies to.
    pub resource: Option<ResourceId>,
    /// Finding severity.
    #[serde(default)]
    pub severity: Severity,
    /// Evaluation result.
    pub result: PolicyResult,
    /// Human-readable explanation with secret values removed.
    pub message: String,
    /// Time the policy engine observed the resource.
    #[schemars(with = "Option<String>")]
    pub observed_at: Option<DateTime<Utc>>,
    /// Engine-specific details with secret values removed.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

/// Counts for a policy report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicySummary {
    /// Passing results.
    #[serde(default)]
    pub pass: u32,
    /// Failing results.
    #[serde(default)]
    pub fail: u32,
    /// Warning results.
    #[serde(default)]
    pub warn: u32,
    /// Error results.
    #[serde(default)]
    pub error: u32,
    /// Skipped results.
    #[serde(default)]
    pub skip: u32,
}

/// Aggregated policy findings for one resource or the whole topology.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyReport {
    /// Resource the report applies to. `None` means the whole topology.
    pub resource: Option<ResourceId>,
    /// Individual findings.
    #[serde(default)]
    pub results: Vec<PolicyRecord>,
    /// Result counts.
    #[serde(default)]
    pub summary: PolicySummary,
    /// Time the report was generated.
    #[schemars(with = "Option<String>")]
    pub generated_at: Option<DateTime<Utc>>,
}

/// Result of `policy.validate_manifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationResult {
    /// True when the API server accepted the manifest.
    pub valid: bool,
    /// YAML returned by the server-side dry run, with defaults applied.
    pub mutated_manifest: Option<String>,
    /// Policy failures that block or warn about the change.
    #[serde(default)]
    pub violations: Vec<PolicyRecord>,
    /// Non-fatal validation warnings.
    #[serde(default)]
    pub warnings: Vec<Warning>,
}
