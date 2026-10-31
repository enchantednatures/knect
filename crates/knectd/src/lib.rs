//! `knectd` daemon library.
//!
//! The binary in `main.rs` only parses flags, initializes logging, and starts
//! the supervised tasks. Everything testable lives here so `cargo test
//! --workspace --lib` exercises the dispatcher, router, SSE framing, UDS
//! listener, and fixture loader without spawning a process.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod error;
pub mod fixture;
pub mod logging;
pub mod rpc;
pub mod runtime;
pub mod transport;
pub mod webui;

/// Daemon version reported by `daemon.status` and `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use app::AppState;
pub use rpc::RpcDispatcher;
