//! End-to-end harness library.
//!
//! Foundation skeleton: the RPC client abstraction, wait/assert helpers, and
//! scenario registry compile and run. Each scenario prints `pending` and
//! succeeds until its owning task fills in real assertions.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod harness;
pub mod scenarios;

pub use harness::{Harness, HarnessError};
pub use scenarios::{SCENARIOS, ScenarioError, run_all, run_scenario};

/// Installs the process-wide rustls provider.
///
/// The workspace builds reqwest with `rustls-no-provider`, so binaries and
/// tests must install the ring provider before constructing a TLS client.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Top-level harness failure.
#[derive(Debug, thiserror::Error)]
pub enum E2eError {
    /// Harness construction or RPC failure.
    #[error(transparent)]
    Harness(#[from] HarnessError),
    /// Scenario failure.
    #[error(transparent)]
    Scenario(#[from] ScenarioError),
}
