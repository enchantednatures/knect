//! Live CloudEvent tap and log subscription payloads.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Resource kind that can be tapped.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TapTargetKind {
    Broker,
    Trigger,
    #[serde(other)]
    Unknown,
}

/// Broker or Trigger targeted by a tap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapTarget {
    /// Target kind.
    pub kind: TapTargetKind,
    /// Target namespace.
    pub namespace: String,
    /// Target name.
    pub name: String,
}

/// Lifecycle state of a tap.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TapState {
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
    Expired,
    #[serde(other)]
    Unknown,
}

/// CloudEvent attribute filter applied to a tap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapFilter {
    /// Exact-match CloudEvent attribute filters.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

/// Handle describing an active or completed tap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapHandle {
    /// Daemon-assigned tap ID.
    pub id: String,
    /// Broker or Trigger being tapped.
    pub target: TapTarget,
    /// Current lifecycle state.
    pub state: TapState,
    /// Creation time.
    #[schemars(with = "String")]
    pub created: DateTime<Utc>,
    /// Time after which the daemon removes the tap.
    #[schemars(with = "String")]
    pub expires: DateTime<Utc>,
    /// Maximum number of events retained for this tap.
    pub max_events: u32,
    /// Callback URL of the daemon's token-guarded receiver.
    pub callback_url: String,
    /// Attribute filter, when one was requested.
    pub filter: Option<TapFilter>,
}

/// One observed CloudEvent delivered to a tap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TapEvent {
    /// Tap-local event ID.
    pub id: String,
    /// CloudEvent `id` attribute.
    pub ce_id: String,
    /// CloudEvent `source` attribute.
    pub ce_source: String,
    /// CloudEvent `type` attribute.
    pub ce_type: String,
    /// CloudEvent `subject` attribute.
    pub ce_subject: Option<String>,
    /// CloudEvent `time` attribute.
    #[schemars(with = "Option<String>")]
    pub ce_time: Option<DateTime<Utc>>,
    /// HTTP headers observed on delivery, with credentials redacted.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// CloudEvent data payload.
    pub payload: serde_json::Value,
    /// Time the daemon received the event.
    #[schemars(with = "String")]
    pub received_at: DateTime<Utc>,
}

/// Stream a log line was read from.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// A pod reference used by `logs.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PodRef {
    /// Pod namespace.
    pub namespace: String,
    /// Pod name.
    pub name: String,
    /// Container name. `None` means the pod's default container.
    pub container: Option<String>,
}

/// One log line delivered by `logs.subscribe` or `log.line` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogLine {
    /// Pod namespace.
    pub namespace: String,
    /// Pod name.
    pub pod: String,
    /// Container name.
    pub container: String,
    /// Log timestamp reported by the kubelet.
    #[schemars(with = "String")]
    pub timestamp: DateTime<Utc>,
    /// Log message with secret values redacted.
    pub message: String,
    /// stdout or stderr.
    pub stream: LogStream,
}
