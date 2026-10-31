//! JSON-RPC 2.0 envelopes, method names, and typed params/results.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::ActionResult;
use crate::graph::{NodeDetails, Topology};
use crate::ids::{Gvk, ResourceId};
use crate::mutation::{MutationDiff, MutationDiffMode};
use crate::policy::ValidationResult;
use crate::tap::{PodRef, TapFilter, TapHandle, TapTarget};

pub use crate::error::{
    ACTIONS_DISABLED, CACHE_CORRUPT, CANCELLED, CONFLICT, ERROR_CODE_NAMES, ErrorData,
    INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, KUBE_UNAVAILABLE, METHOD_NOT_FOUND, NOT_FOUND,
    PARSE_ERROR, POLICY_BLOCKED, REDACTION_VIOLATION, RpcError, TAP_FORBIDDEN, TAP_LIMIT, TIMEOUT,
    UNSUPPORTED_GVK,
};

/// JSON-RPC protocol version sent in every envelope.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC request ID. Numbers, strings, and `null` are all legal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RpcId {
    /// Numeric request ID.
    Number(i64),
    /// String request ID.
    String(String),
    /// Null request ID (notifications).
    #[default]
    Null,
}

impl RpcId {
    /// Returns true when this ID marks a notification.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<i64> for RpcId {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl From<String> for RpcId {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RpcId {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

/// JSON-RPC request envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RpcRequest {
    /// Must be [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// Request ID. Missing IDs decode as `null` (notification).
    #[serde(default)]
    pub id: RpcId,
    /// Method name. See [`methods`].
    pub method: String,
    /// Method-specific parameters.
    #[serde(default)]
    pub params: Option<Value>,
}

impl RpcRequest {
    /// Creates a request with an explicit ID.
    #[must_use]
    pub fn new(id: impl Into<RpcId>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }

    /// Creates a notification request with a `null` ID.
    #[must_use]
    pub fn notification(method: impl Into<String>, params: Option<Value>) -> Self {
        Self::new(RpcId::Null, method, params)
    }

    /// Returns true when this request has a `null` ID.
    #[must_use]
    pub fn is_notification(&self) -> bool {
        self.id.is_null()
    }
}

/// JSON-RPC response envelope. Exactly one of `result` and `error` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RpcResponse {
    /// Must be [`JSONRPC_VERSION`].
    pub jsonrpc: String,
    /// ID copied from the request. `null` when the request ID was unknown.
    #[serde(default)]
    pub id: RpcId,
    /// Successful result.
    #[serde(default)]
    pub result: Option<Value>,
    /// Error result.
    #[serde(default)]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    /// Creates a successful response.
    #[must_use]
    pub fn success(id: impl Into<RpcId>, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

    /// Creates an error response.
    #[must_use]
    pub fn failure(id: impl Into<RpcId>, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: id.into(),
            result: None,
            error: Some(error),
        }
    }

    /// Returns true when the response carries an error.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Params for methods that take an empty object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmptyParams {}

/// Result for methods that return an empty object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmptyResult {}

/// A watcher's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WatcherStatus {
    /// Watched GVK.
    pub gvk: Gvk,
    /// Number of objects observed through this watcher.
    pub resource_count: u64,
    /// True when the last watch or relist succeeded.
    pub healthy: bool,
    /// Time of the most recent watch event.
    #[schemars(with = "Option<String>")]
    pub last_event: Option<DateTime<Utc>>,
}

/// Result of `daemon.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
    /// Daemon version.
    pub version: String,
    /// Protocol schema version.
    pub schema_version: u32,
    /// Kubernetes context name.
    pub context: String,
    /// Current topology generation.
    pub generation: u64,
    /// Active watchers and their health.
    #[serde(default)]
    pub watchers: Vec<WatcherStatus>,
    /// Seconds since the daemon started.
    pub uptime_secs: u64,
    /// Whether guarded write actions are enabled.
    pub actions_enabled: bool,
    /// Number of active taps.
    pub tap_count: u32,
    /// Daemon start time.
    #[schemars(with = "Option<String>")]
    pub started_at: Option<DateTime<Utc>>,
    /// Whether the snapshot cache is enabled.
    pub cache_enabled: bool,
    /// Time of the last successful discovery resync.
    #[schemars(with = "Option<String>")]
    pub last_resync: Option<DateTime<Utc>>,
}

/// Params for `daemon.status`.
pub type DaemonStatusParams = EmptyParams;
/// Result alias for `daemon.status`.
pub type DaemonStatusResult = DaemonStatus;

/// Optional sections of a topology response.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TopologySection {
    Nodes,
    Edges,
    Warnings,
    Stats,
}

/// Params for `graph.get_topology`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetTopologyParams {
    /// Kubernetes context. Defaults to the daemon's configured context.
    pub context: Option<String>,
    /// Wait briefly until this generation is available.
    pub min_generation: Option<u64>,
    /// Optional section filter. Omitted means all sections.
    pub include: Option<Vec<TopologySection>>,
}

/// Result alias for `graph.get_topology`.
pub type GetTopologyResult = Topology;

/// Params for `graph.get_node_details`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetNodeDetailsParams {
    /// Node to inspect.
    pub id: ResourceId,
    /// Include redacted YAML.
    pub include_yaml: Option<bool>,
    /// Include recent log lines.
    pub include_logs: Option<bool>,
}

/// Result alias for `graph.get_node_details`.
pub type GetNodeDetailsResult = NodeDetails;

/// Params for `graph.refresh`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefreshParams {
    /// Force re-discovery of API resources.
    pub rediscover: Option<bool>,
}

/// Result of `graph.refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefreshResult {
    /// Generation reached after the refresh.
    pub generation: u64,
}

/// Params for `mutations.get_diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetDiffParams {
    /// Resource to attribute.
    pub id: ResourceId,
    /// Diff mode. Defaults to `three_way`.
    pub mode: Option<MutationDiffMode>,
}

/// Result alias for `mutations.get_diff`.
pub type GetDiffResult = MutationDiff;

/// One revision's share of service traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficSplit {
    /// Revision name.
    pub revision: String,
    /// Percentage of traffic. All percentages in a request must sum to 100.
    pub percent: u8,
    /// Optional traffic tag.
    pub tag: Option<String>,
}

/// Params for `actions.set_traffic_split`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetTrafficSplitParams {
    /// Service namespace.
    pub namespace: String,
    /// Knative Service name.
    pub service: String,
    /// Desired traffic split.
    pub splits: Vec<TrafficSplit>,
    /// Validate without writing.
    pub dry_run: Option<bool>,
}

/// Result alias for `actions.set_traffic_split`.
pub type SetTrafficSplitResult = ActionResult;

/// Flagger canary operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlaggerAction {
    Promote,
    Abort,
}

/// Params for `actions.flagger_control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FlaggerControlParams {
    /// Canary namespace.
    pub namespace: String,
    /// Canary name.
    pub canary: String,
    /// Promote or abort.
    pub action: FlaggerAction,
    /// Validate without writing.
    pub dry_run: Option<bool>,
}

/// Result alias for `actions.flagger_control`.
pub type FlaggerControlResult = ActionResult;

/// Params for `policy.validate_manifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidateManifestParams {
    /// YAML manifest to validate.
    pub manifest: String,
    /// Namespace applied when the manifest omits one.
    pub namespace: Option<String>,
    /// Default kind applied when the manifest omits one.
    pub default_kind: Option<String>,
}

/// Result alias for `policy.validate_manifest`.
pub type ValidateManifestResult = ValidationResult;

/// Params for `tap.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapStartParams {
    /// Broker or Trigger to tap.
    pub target: TapTarget,
    /// Optional CloudEvent attribute filter.
    pub filter: Option<TapFilter>,
    /// Maximum number of events to retain.
    pub max_events: Option<u32>,
    /// Time-to-live in seconds.
    pub ttl_secs: Option<u64>,
}

/// Result alias for `tap.start`.
pub type TapStartResult = TapHandle;

/// Params for `tap.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapStopParams {
    /// Tap ID returned by `tap.start`.
    pub tap_id: String,
}

/// Result of `tap.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TapStopResult {
    /// True when an active tap was removed. Idempotent stops return `false`.
    pub removed: bool,
}

/// Params for `tap.list`.
pub type TapListParams = EmptyParams;
/// Result for `tap.list`.
pub type TapListResult = Vec<TapHandle>;

/// Params for `logs.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsSubscribeParams {
    /// Pods to follow.
    pub pods: Vec<PodRef>,
    /// Number of historical lines to read.
    pub tail_lines: Option<u32>,
    /// Start time for historical lines.
    #[schemars(with = "Option<String>")]
    pub since: Option<DateTime<Utc>>,
    /// Optional substring or regular-expression filter.
    pub filter: Option<String>,
}

/// Result of `logs.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsSubscribeResult {
    /// Subscription ID used by `logs.stop`.
    pub subscription_id: String,
}

/// Params for `logs.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsStopParams {
    /// Subscription ID returned by `logs.subscribe`.
    pub subscription_id: String,
}

/// Result of `logs.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsStopResult {
    /// True when an active subscription was stopped.
    pub stopped: bool,
}

/// Params for `events.subscribe`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsSubscribeParams {
    /// Event topics to receive. Omitted means all topics.
    pub topics: Option<Vec<String>>,
    /// Resume after this SSE sequence number.
    pub last_event_id: Option<u64>,
}

/// Result of `events.subscribe` over newline-delimited JSON-RPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsSubscribeResult {
    /// Subscription ID used to correlate the JSONL event stream.
    pub subscription_id: String,
}

/// Params for `config.get`.
pub type ConfigGetParams = EmptyParams;

/// Safe daemon configuration returned by `config.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeConfig {
    /// Daemon transport settings.
    pub daemon: SafeDaemonConfig,
    /// Discovery settings.
    pub discovery: SafeDiscoveryConfig,
    /// Watch settings.
    pub watch: SafeWatchConfig,
    /// Cache settings.
    pub cache: SafeCacheConfig,
    /// Tap settings.
    pub tap: SafeTapConfig,
    /// UI settings.
    pub ui: SafeUiConfig,
    /// Action settings.
    pub actions: SafeActionsConfig,
}

/// Safe daemon transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeDaemonConfig {
    /// Unix domain socket path.
    pub socket_path: String,
    /// HTTP listen address.
    pub http_addr: String,
    /// Cache directory.
    pub cache_dir: String,
    /// Kubernetes context name.
    pub context: String,
    /// Log level filter.
    pub log: String,
    /// Log format (`human` or `json`).
    pub log_format: String,
    /// Whether write actions are enabled.
    pub allow_actions: bool,
    /// Whether remote HTTP clients are allowed.
    pub allow_remote: bool,
}

/// Safe discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeDiscoveryConfig {
    /// Rediscovery interval in seconds.
    pub rediscover_secs: u64,
    /// API groups that are discovered.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Namespaces watched. Empty means all namespaces.
    #[serde(default)]
    pub namespaces: Vec<String>,
}

/// Safe watch settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeWatchConfig {
    /// Recompute debounce in milliseconds.
    pub debounce_ms: u64,
    /// Maximum number of tracked objects.
    pub max_objects: u64,
    /// Annotation keys stripped before objects enter the store.
    #[serde(default)]
    pub strip_annotations: Vec<String>,
}

/// Safe cache settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeCacheConfig {
    /// Whether the snapshot cache is enabled.
    pub enabled: bool,
    /// Flush interval in milliseconds.
    pub flush_ms: u64,
    /// Maximum serialized snapshot size in bytes.
    pub max_snapshot_bytes: u64,
}

/// Safe tap settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeTapConfig {
    /// Whether taps are enabled.
    pub enabled: bool,
    /// Callback listener bind address.
    pub callback_bind: String,
    /// Callback host advertised to Knative.
    pub callback_host: String,
    /// Maximum concurrent taps.
    pub max_taps: u32,
    /// Maximum events retained per tap.
    pub max_events: u32,
    /// Tap time-to-live in seconds.
    pub ttl_secs: u64,
}

/// Safe UI settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeUiConfig {
    /// Target frames per second.
    pub fps: u32,
    /// Theme name.
    pub theme: String,
    /// Whether to use ASCII-only rendering.
    pub ascii: bool,
}

/// Safe action settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SafeActionsConfig {
    /// Whether guarded write actions are enabled.
    pub enabled: bool,
}

/// Result alias for `config.get`.
pub type ConfigGetResult = SafeConfig;

/// Frozen JSON-RPC method names.
pub mod methods {
    /// `daemon.status`
    pub const DAEMON_STATUS: &str = "daemon.status";
    /// `graph.get_topology`
    pub const GRAPH_GET_TOPOLOGY: &str = "graph.get_topology";
    /// `graph.get_node_details`
    pub const GRAPH_GET_NODE_DETAILS: &str = "graph.get_node_details";
    /// `graph.refresh`
    pub const GRAPH_REFRESH: &str = "graph.refresh";
    /// `mutations.get_diff`
    pub const MUTATIONS_GET_DIFF: &str = "mutations.get_diff";
    /// `actions.set_traffic_split`
    pub const ACTIONS_SET_TRAFFIC_SPLIT: &str = "actions.set_traffic_split";
    /// `actions.flagger_control`
    pub const ACTIONS_FLAGGER_CONTROL: &str = "actions.flagger_control";
    /// `policy.validate_manifest`
    pub const POLICY_VALIDATE_MANIFEST: &str = "policy.validate_manifest";
    /// `tap.start`
    pub const TAP_START: &str = "tap.start";
    /// `tap.stop`
    pub const TAP_STOP: &str = "tap.stop";
    /// `tap.list`
    pub const TAP_LIST: &str = "tap.list";
    /// `logs.subscribe`
    pub const LOGS_SUBSCRIBE: &str = "logs.subscribe";
    /// `logs.stop`
    pub const LOGS_STOP: &str = "logs.stop";
    /// `events.subscribe`
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";
    /// `config.get`
    pub const CONFIG_GET: &str = "config.get";

    /// Every method name, in contract order.
    pub const ALL: &[&str] = &[
        DAEMON_STATUS,
        GRAPH_GET_TOPOLOGY,
        GRAPH_GET_NODE_DETAILS,
        GRAPH_REFRESH,
        MUTATIONS_GET_DIFF,
        ACTIONS_SET_TRAFFIC_SPLIT,
        ACTIONS_FLAGGER_CONTROL,
        POLICY_VALIDATE_MANIFEST,
        TAP_START,
        TAP_STOP,
        TAP_LIST,
        LOGS_SUBSCRIBE,
        LOGS_STOP,
        EVENTS_SUBSCRIBE,
        CONFIG_GET,
    ];
}
