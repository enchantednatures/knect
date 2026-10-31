//! Typed error model shared by every core subsystem.
//!
//! Every fallible core API returns [`CoreError`]. The daemon maps it to the
//! frozen JSON-RPC error contract with [`CoreError::to_rpc_error`]; the `code`
//! and `retryable` mappings are part of the foundation contract and must stay
//! stable.

use knect_proto::error::{
    ACTIONS_DISABLED, CACHE_CORRUPT, INTERNAL_ERROR, INVALID_PARAMS, KUBE_UNAVAILABLE,
    POLICY_BLOCKED, TAP_LIMIT, UNSUPPORTED_GVK,
};
use knect_proto::{ErrorData, RpcError};

/// Errors produced by `knect-core`.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// API discovery failed.
    #[error("discovery failed: {0}")]
    Discovery(String),
    /// A watch stream failed or was interrupted.
    #[error("watch failed: {0}")]
    Watch(String),
    /// Graph construction failed.
    #[error("graph error: {0}")]
    Graph(String),
    /// Snapshot cache failed.
    #[error("cache error: {0}")]
    Cache(String),
    /// The Kubernetes API returned an error.
    #[error("kubernetes error: {0}")]
    Kube(#[from] kube::Error),
    /// A policy engine blocked the operation.
    #[error("policy error: {0}")]
    Policy(String),
    /// A guarded write action failed or is disabled.
    #[error("action error: {0}")]
    Action(String),
    /// A tap operation failed.
    #[error("tap error: {0}")]
    Tap(String),
    /// Configuration is invalid or could not be loaded.
    #[error("configuration error: {0}")]
    Config(String),
    /// Resource store invariant was violated.
    #[error("store error: {0}")]
    Store(String),
    /// Serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// The requested capability is not implemented in this build.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// An unexpected internal failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl CoreError {
    /// Maps this error to the frozen JSON-RPC error code.
    #[must_use]
    pub const fn code(&self) -> i32 {
        match self {
            Self::Discovery(_) | Self::Watch(_) | Self::Kube(_) => KUBE_UNAVAILABLE,
            Self::Graph(_) | Self::Store(_) | Self::Serialization(_) | Self::Internal(_) => {
                INTERNAL_ERROR
            }
            Self::Cache(_) => CACHE_CORRUPT,
            Self::Policy(_) => POLICY_BLOCKED,
            Self::Action(_) => ACTIONS_DISABLED,
            Self::Tap(_) => TAP_LIMIT,
            Self::Config(_) => INVALID_PARAMS,
            Self::Unsupported(_) => UNSUPPORTED_GVK,
        }
    }

    /// Returns true when retrying the same operation may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Discovery(_) | Self::Watch(_) | Self::Kube(_))
    }

    /// Builds the protocol error object for this failure.
    #[must_use]
    pub fn to_rpc_error(&self) -> RpcError {
        RpcError::with_data(
            self.code(),
            self.to_string(),
            ErrorData {
                retryable: self.retryable(),
                details: None,
                resource: None,
            },
        )
    }

    /// Creates a not-found error for a resource identifier.
    #[must_use]
    pub fn not_found(resource: knect_proto::ResourceId) -> Self {
        Self::Store(format!("{resource} is not present in the store"))
    }
}

impl From<CoreError> for RpcError {
    fn from(error: CoreError) -> Self {
        error.to_rpc_error()
    }
}

/// Convenience alias for core results.
pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_the_protocol_contract() {
        assert_eq!(CoreError::Discovery("x".into()).code(), KUBE_UNAVAILABLE);
        assert_eq!(CoreError::Watch("x".into()).code(), KUBE_UNAVAILABLE);
        assert_eq!(CoreError::Graph("x".into()).code(), INTERNAL_ERROR);
        assert_eq!(CoreError::Cache("x".into()).code(), CACHE_CORRUPT);
        assert_eq!(CoreError::Policy("x".into()).code(), POLICY_BLOCKED);
        assert_eq!(CoreError::Action("x".into()).code(), ACTIONS_DISABLED);
        assert_eq!(CoreError::Tap("x".into()).code(), TAP_LIMIT);
        assert_eq!(CoreError::Config("x".into()).code(), INVALID_PARAMS);
        assert_eq!(CoreError::Store("x".into()).code(), INTERNAL_ERROR);
        assert_eq!(CoreError::Unsupported("x".into()).code(), UNSUPPORTED_GVK);
        assert_eq!(CoreError::Internal("x".into()).code(), INTERNAL_ERROR);
        assert_eq!(
            CoreError::not_found(knect_proto::ResourceId::namespaced(
                "v1", "Pod", "default", "missing"
            ))
            .code(),
            INTERNAL_ERROR
        );
    }

    #[test]
    fn retryability_is_advisory_and_stable() {
        assert!(CoreError::Discovery("x".into()).retryable());
        assert!(CoreError::Watch("x".into()).retryable());
        assert!(!CoreError::Graph("x".into()).retryable());
        assert!(!CoreError::Config("x".into()).retryable());
        assert!(!CoreError::Unsupported("x".into()).retryable());
    }

    #[test]
    fn rpc_error_keeps_code_and_retryable_flag() {
        let error = CoreError::Discovery("no api server".into());
        let rpc = error.to_rpc_error();
        assert_eq!(rpc.code, KUBE_UNAVAILABLE);
        assert!(rpc.data.as_ref().is_some_and(|data| data.retryable));
        assert_eq!(rpc.message, "discovery failed: no api server");

        let converted: RpcError = CoreError::Config("bad toml".into()).into();
        assert_eq!(converted.code, INVALID_PARAMS);
    }

    #[test]
    fn serialization_errors_convert() {
        let error: CoreError = serde_json::from_str::<serde_json::Value>("{")
            .expect_err("invalid JSON must fail")
            .into();
        assert_eq!(error.code(), INTERNAL_ERROR);
        assert!(matches!(error, CoreError::Serialization(_)));
    }
}
