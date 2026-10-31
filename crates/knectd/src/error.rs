//! Daemon-local typed errors.
//!
//! Core failures stay [`knect_core::CoreError`]; transport, fixture, and
//! startup failures use this enum. Binary top-level code may wrap it in
//! `anyhow`, but library paths stay typed.

use knect_core::CoreError;

/// Errors produced by daemon startup and transports.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// Invalid daemon configuration.
    #[error("configuration error: {0}")]
    Config(String),
    /// Listener or connection failure.
    #[error("transport error: {0}")]
    Transport(String),
    /// RPC framing failure.
    #[error("rpc error: {0}")]
    Rpc(String),
    /// Fixture loading failure.
    #[error("fixture error: {0}")]
    Fixture(String),
    /// Runtime supervision failure.
    #[error("runtime error: {0}")]
    Runtime(String),
}

impl From<CoreError> for DaemonError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Config(message) => Self::Config(message),
            other => Self::Runtime(other.to_string()),
        }
    }
}

/// Convenience alias for daemon results.
pub type DaemonResult<T> = Result<T, DaemonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_errors_map_to_daemon_errors() {
        let error: DaemonError = CoreError::Config("bad".to_owned()).into();
        assert!(matches!(error, DaemonError::Config(_)));
        let error: DaemonError = CoreError::Internal("boom".to_owned()).into();
        assert!(matches!(error, DaemonError::Runtime(_)));
    }
}
