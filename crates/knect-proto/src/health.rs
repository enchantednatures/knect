//! Health and scaling summaries derived from workload status.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::graph::Health;

/// Scale-to-zero and autoscaling state for a workload.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ScaleState {
    Active,
    ScalingUp,
    ScaledToZero,
    Degraded,
    #[serde(other)]
    Unknown,
}

/// Aggregated health and scale information for a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HealthSummary {
    /// Roll-up health.
    pub health: Health,
    /// Current scale state.
    pub scale_state: ScaleState,
    /// Ready replicas, when the resource exposes them.
    pub ready_replicas: Option<u32>,
    /// Desired replicas, when the resource exposes them.
    pub desired_replicas: Option<u32>,
    /// Human-readable reasons contributing to the health state.
    #[serde(default)]
    pub reasons: Vec<String>,
    /// Last time the health state changed.
    #[schemars(with = "Option<String>")]
    pub last_transition: Option<DateTime<Utc>>,
}
