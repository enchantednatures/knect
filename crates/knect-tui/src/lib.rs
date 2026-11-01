//! `knect` TUI and CLI.
//!
//! Task 13 provides the four-pane dashboard, the layered DAG renderer, the
//! daemon client (Unix JSONL and HTTP/SSE), and daemon auto-spawn. Task 14
//! adds the mutation/traffic/tap modals and Task 15 fills the CLI subcommands.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod cli;
pub mod client;
pub mod ui;

pub use cli::Cli;
