//! Server-Sent Events envelopes and event payloads.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::{Edge, ResourceNode, Topology, Warning};
use crate::ids::ResourceId;
use crate::policy::PolicyRecord;
use crate::tap::{LogLine, TapEvent};

/// Frozen SSE event names.
pub mod event_names {
    /// Full topology replacement.
    pub const SNAPSHOT: &str = "snapshot";
    /// Incremental graph patch.
    pub const GRAPH_PATCH: &str = "graph.patch";
    /// One node changed.
    pub const NODE_UPDATED: &str = "node.updated";
    /// A policy finding was observed.
    pub const POLICY_VIOLATION: &str = "policy.violation";
    /// A CloudEvent was captured by a tap.
    pub const TAP_EVENT: &str = "tap.event";
    /// A log line was emitted by a subscription.
    pub const LOG_LINE: &str = "log.line";
    /// A write action completed.
    pub const ACTION_RESULT: &str = "action.result";
    /// Daemon liveness signal.
    pub const DAEMON_HEARTBEAT: &str = "daemon.heartbeat";
    /// Daemon-level error.
    pub const DAEMON_ERROR: &str = "daemon.error";
}

/// Every SSE event name, in contract order.
pub const ALL_EVENT_NAMES: &[&str] = &[
    event_names::SNAPSHOT,
    event_names::GRAPH_PATCH,
    event_names::NODE_UPDATED,
    event_names::POLICY_VIOLATION,
    event_names::TAP_EVENT,
    event_names::LOG_LINE,
    event_names::ACTION_RESULT,
    event_names::DAEMON_HEARTBEAT,
    event_names::DAEMON_ERROR,
];

/// Incremental topology change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphPatch {
    /// New topology generation.
    pub generation: u64,
    /// Nodes added by this generation.
    #[serde(default)]
    pub added_nodes: Vec<ResourceNode>,
    /// Nodes whose rendered content changed.
    #[serde(default)]
    pub updated_nodes: Vec<ResourceNode>,
    /// Node IDs removed by this generation.
    #[serde(default)]
    pub removed_nodes: Vec<ResourceId>,
    /// Edges added by this generation.
    #[serde(default)]
    pub added_edges: Vec<Edge>,
    /// Edge IDs removed by this generation.
    #[serde(default)]
    pub removed_edges: Vec<String>,
    /// Monotonic event sequence number.
    pub seq: u64,
    /// Time the patch was produced.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// A single node changed without a full graph patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NodeUpdated {
    /// Updated node projection.
    pub node: ResourceNode,
    /// Names of the fields that changed.
    #[serde(default)]
    pub changed_fields: Vec<String>,
    /// Topology generation after the change.
    pub generation: u64,
    /// Time the update was observed.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// A policy finding became visible or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolicyViolation {
    /// Policy finding.
    pub policy: PolicyRecord,
    /// Topology generation after the change.
    pub generation: u64,
    /// Time the finding was observed.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// Result of a guarded write action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionResult {
    /// Method or action name.
    pub action: String,
    /// True when the action completed.
    pub success: bool,
    /// Human-readable outcome with secret values removed.
    pub message: String,
    /// True when the action was only validated.
    pub dry_run: bool,
    /// Resource affected by the action.
    pub resource: Option<ResourceId>,
    /// Topology generation after the action.
    pub generation: Option<u64>,
    /// Non-fatal warnings emitted by the action.
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// Time the action completed.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// Daemon liveness payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Heartbeat {
    /// Seconds since the daemon started.
    pub uptime_secs: u64,
    /// Current topology generation.
    pub generation: u64,
    /// Current node count.
    pub node_count: u64,
    /// Current edge count.
    pub edge_count: u64,
    /// Time the heartbeat was produced.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// Daemon-level error delivered on the event stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonError {
    /// Numeric error code from [`crate::error`].
    pub code: i32,
    /// Short human-readable message.
    pub message: String,
    /// True when reconnecting or retrying may succeed.
    pub retryable: bool,
    /// Time the error occurred.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
}

/// Tagged event payload carried by [`SseEvent`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum SseEventKind {
    /// Full topology replacement.
    #[serde(rename = "snapshot")]
    Snapshot {
        /// Replacement topology.
        data: Topology,
    },
    /// Incremental graph patch.
    #[serde(rename = "graph.patch")]
    GraphPatch {
        /// Patch contents.
        data: GraphPatch,
    },
    /// One node changed.
    #[serde(rename = "node.updated")]
    NodeUpdated {
        /// Updated node.
        data: NodeUpdated,
    },
    /// Policy finding.
    #[serde(rename = "policy.violation")]
    PolicyViolation {
        /// Finding contents.
        data: PolicyViolation,
    },
    /// CloudEvent captured by a tap.
    #[serde(rename = "tap.event")]
    TapEvent {
        /// Captured event.
        data: TapEvent,
    },
    /// Log line.
    #[serde(rename = "log.line")]
    LogLine {
        /// Log line contents.
        data: LogLine,
    },
    /// Write-action result.
    #[serde(rename = "action.result")]
    ActionResult {
        /// Action result contents.
        data: ActionResult,
    },
    /// Daemon heartbeat.
    #[serde(rename = "daemon.heartbeat")]
    Heartbeat {
        /// Heartbeat contents.
        data: Heartbeat,
    },
    /// Daemon error.
    #[serde(rename = "daemon.error")]
    DaemonError {
        /// Error contents.
        data: DaemonError,
    },
}

impl SseEventKind {
    /// Returns the frozen SSE event name for this payload.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Snapshot { .. } => event_names::SNAPSHOT,
            Self::GraphPatch { .. } => event_names::GRAPH_PATCH,
            Self::NodeUpdated { .. } => event_names::NODE_UPDATED,
            Self::PolicyViolation { .. } => event_names::POLICY_VIOLATION,
            Self::TapEvent { .. } => event_names::TAP_EVENT,
            Self::LogLine { .. } => event_names::LOG_LINE,
            Self::ActionResult { .. } => event_names::ACTION_RESULT,
            Self::Heartbeat { .. } => event_names::DAEMON_HEARTBEAT,
            Self::DaemonError { .. } => event_names::DAEMON_ERROR,
        }
    }
}

/// SSE frame data. Clients receive `id: <seq>`, `event: <event_type>`, and
/// `data: <this JSON>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SseEvent {
    /// Monotonic event sequence number, also sent as the SSE `id`.
    pub seq: u64,
    /// Time the event was produced.
    #[schemars(with = "String")]
    pub ts: DateTime<Utc>,
    /// Tagged event payload. Flattened so JSON contains a `kind` field.
    #[serde(flatten)]
    pub kind: SseEventKind,
}

impl SseEvent {
    /// Creates an SSE event.
    #[must_use]
    pub fn new(seq: u64, ts: DateTime<Utc>, kind: SseEventKind) -> Self {
        Self { seq, ts, kind }
    }

    /// Returns the frozen SSE event name.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        self.kind.event_type()
    }
}
