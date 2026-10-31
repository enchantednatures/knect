//! Discovered links between resources and their redacted evidence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::Health;
use crate::ids::{FieldPath, ResourceId};

/// How a link was discovered.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LinkVia {
    /// Knative `spec.configuration` / `spec.routes` style explicit binding.
    ExplicitBinding,
    /// Environment variable value.
    Env,
    /// `envFrom` or Secret mount reference.
    Secret,
    /// ConfigMap key reference.
    ConfigMap,
    /// DNS name observed in an environment value or endpoint address.
    Dns,
    /// Backing operator CRD relationship.
    OperatorCrd,
    /// PVC/volume relationship.
    Storage,
}

/// Evidence kind for a discovered link.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    EnvVar,
    EnvFrom,
    SecretKey,
    ConfigMapKey,
    DnsName,
    Uri,
    Port,
    OwnerReference,
    Field,
    #[serde(other)]
    Other,
}

/// One piece of redacted evidence supporting a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    /// What kind of source material produced the evidence.
    pub kind: EvidenceKind,
    /// Human-readable explanation with secret values removed.
    pub description: String,
    /// Field path where the evidence was found, when known.
    pub path: Option<FieldPath>,
    /// Redacted value preview. Raw secret and ConfigMap values never appear here.
    pub value_redacted: Option<String>,
}

/// A discovered relationship with confidence and evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinkRecord {
    /// Resource that references or depends on the target.
    pub source: ResourceId,
    /// Resource being referenced.
    pub target: ResourceId,
    /// Discovery mechanism.
    pub via: LinkVia,
    /// Confidence in the range `0.0..=1.0`.
    pub confidence: f64,
    /// Evidence that produced this link, with values redacted.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// Health of the link.
    pub health: Health,
}
