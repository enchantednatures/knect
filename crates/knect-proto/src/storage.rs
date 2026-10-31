//! Persistent storage tracing and RWO conflict warnings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::Severity;
use crate::ids::ResourceId;

/// Kubernetes persistent volume access mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadWriteOnce,
    ReadOnlyMany,
    ReadWriteMany,
    ReadWriteOncePod,
    #[serde(other)]
    Unknown,
}

/// A warning about a persistent volume claim or its consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StorageWarning {
    /// Stable machine-readable warning code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Warning severity.
    #[serde(default)]
    pub severity: Severity,
    /// Detailed conflict when this warning represents a read-write-once conflict.
    pub conflict: Option<StorageConflictWarning>,
}

/// Read-write-once claim mounted by more than one concurrent consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StorageConflictWarning {
    /// Claim involved in the conflict.
    pub pvc: ResourceId,
    /// Access mode that triggered the warning.
    pub access_mode: AccessMode,
    /// Highest replica count observed across consumers, when known.
    pub max_scale: Option<u32>,
    /// Explanation of why this is a conflict.
    pub reason: String,
    /// Resources mounting the claim.
    #[serde(default)]
    pub affected: Vec<ResourceId>,
}

/// Storage relationship traced from a workload to its backing volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StorageRecord {
    /// Persistent volume claim.
    pub pvc: ResourceId,
    /// Volume name inside the pod spec.
    pub volume: Option<String>,
    /// StorageClass name.
    pub storage_class: Option<String>,
    /// Access modes requested by the claim.
    #[serde(default)]
    pub access_modes: Vec<AccessMode>,
    /// Requested or actual capacity, for example `20Gi`.
    pub capacity: Option<String>,
    /// Claim phase, for example `Bound`.
    pub phase: Option<String>,
    /// Resources that mount this claim.
    #[serde(default)]
    pub mounted_by: Vec<ResourceId>,
    /// StorageClass workload that provisioned the claim.
    pub provisioned_by: Option<ResourceId>,
    /// Backing operator resource, for example a Ceph pool or database cluster.
    pub backend: Option<ResourceId>,
    /// Storage-specific warnings.
    #[serde(default)]
    pub warnings: Vec<StorageWarning>,
}
