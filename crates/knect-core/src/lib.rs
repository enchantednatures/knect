//! Core engine for `knect`.
//!
//! This crate is a placeholder until the discovery, graph, cache, and action
//! implementations land. It intentionally exposes only the crate version.

/// Returns the version of the `knect-core` crate compiled into the binary.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
