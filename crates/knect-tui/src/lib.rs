//! `knect` TUI and CLI.
//!
//! The foundation milestone freezes the CLI surface, client trait, app state
//! shell, and placeholder UI modules. Task 13 replaces the placeholder render
//! path with the four-pane product; Task 15 fills the CLI subcommands.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod cli;
pub mod client;
pub mod ui;

pub use cli::Cli;
