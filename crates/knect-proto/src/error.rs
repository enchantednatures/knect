//! JSON-RPC error model shared by the daemon, clients, and plugins.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::ResourceId;

/// JSON-RPC parse error.
pub const PARSE_ERROR: i32 = -32700;
/// JSON-RPC invalid request.
pub const INVALID_REQUEST: i32 = -32600;
/// JSON-RPC method not found.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC invalid params.
pub const INVALID_PARAMS: i32 = -32602;
/// JSON-RPC internal error.
pub const INTERNAL_ERROR: i32 = -32603;
/// Application error: resource not found.
pub const NOT_FOUND: i32 = -32001;
/// Application error: conflicting concurrent modification.
pub const CONFLICT: i32 = -32002;
/// Application error: Kubernetes API unavailable.
pub const KUBE_UNAVAILABLE: i32 = -32003;
/// Application error: a policy blocked the action.
pub const POLICY_BLOCKED: i32 = -32004;
/// Application error: GVK not supported by this daemon.
pub const UNSUPPORTED_GVK: i32 = -32005;
/// Application error: write actions are disabled.
pub const ACTIONS_DISABLED: i32 = -32006;
/// Application error: tap limit reached.
pub const TAP_LIMIT: i32 = -32007;
/// Application error: tap callback forbidden.
pub const TAP_FORBIDDEN: i32 = -32008;
/// Application error: cache snapshot is corrupt.
pub const CACHE_CORRUPT: i32 = -32009;
/// Application error: redaction could not be guaranteed.
pub const REDACTION_VIOLATION: i32 = -32010;
/// Application error: operation timed out.
pub const TIMEOUT: i32 = -32011;
/// Application error: operation cancelled.
pub const CANCELLED: i32 = -32012;

/// Every application and protocol error code, with a stable diagnostic name.
pub const ERROR_CODE_NAMES: &[(&str, i32)] = &[
    ("parse_error", PARSE_ERROR),
    ("invalid_request", INVALID_REQUEST),
    ("method_not_found", METHOD_NOT_FOUND),
    ("invalid_params", INVALID_PARAMS),
    ("internal_error", INTERNAL_ERROR),
    ("not_found", NOT_FOUND),
    ("conflict", CONFLICT),
    ("kube_unavailable", KUBE_UNAVAILABLE),
    ("policy_blocked", POLICY_BLOCKED),
    ("unsupported_gvk", UNSUPPORTED_GVK),
    ("actions_disabled", ACTIONS_DISABLED),
    ("tap_limit", TAP_LIMIT),
    ("tap_forbidden", TAP_FORBIDDEN),
    ("cache_corrupt", CACHE_CORRUPT),
    ("redaction_violation", REDACTION_VIOLATION),
    ("timeout", TIMEOUT),
    ("cancelled", CANCELLED),
];

/// Machine-readable error context attached to a JSON-RPC error object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorData {
    /// True when retrying the same request may succeed.
    #[serde(default)]
    pub retryable: bool,
    /// Additional diagnostic detail with secret values removed.
    pub details: Option<String>,
    /// Resource involved in the error, when known.
    pub resource: Option<ResourceId>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{message} (code {code})")]
pub struct RpcError {
    /// Numeric error code.
    pub code: i32,
    /// Short human-readable message.
    pub message: String,
    /// Optional machine-readable context.
    pub data: Option<ErrorData>,
}

impl RpcError {
    /// Creates an error without context data.
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Creates an error with context data.
    #[must_use]
    pub fn with_data(code: i32, message: impl Into<String>, data: ErrorData) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Creates a not-found error for a resource.
    #[must_use]
    pub fn resource_not_found(resource: ResourceId) -> Self {
        Self::with_data(
            NOT_FOUND,
            "resource not found",
            ErrorData {
                retryable: false,
                details: Some(format!("{resource} is not present in the current graph")),
                resource: Some(resource),
            },
        )
    }

    /// Creates a policy-blocked error.
    #[must_use]
    pub fn policy_blocked(details: impl Into<String>) -> Self {
        Self::with_data(
            POLICY_BLOCKED,
            "policy blocked the requested action",
            ErrorData {
                retryable: false,
                details: Some(details.into()),
                resource: None,
            },
        )
    }

    /// Creates the error returned when the client and daemon schema versions differ.
    #[must_use]
    pub fn schema_version_mismatch(found: u32, expected: u32) -> Self {
        Self::with_data(
            INVALID_PARAMS,
            "schema version mismatch",
            ErrorData {
                retryable: false,
                details: Some(format!(
                    "client schema_version {found}, daemon schema_version {expected}"
                )),
                resource: None,
            },
        )
    }
}

/// Builds an [`RpcError`] from its parts.
#[must_use]
pub fn into_rpc_error(code: i32, message: impl Into<String>, data: ErrorData) -> RpcError {
    RpcError::with_data(code, message, data)
}
