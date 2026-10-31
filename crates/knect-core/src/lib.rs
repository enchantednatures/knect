//! Core engine for `knect`.
//!
//! `knect-core` owns everything that is independent of the daemon transport:
//! configuration, the Kubernetes reader abstraction, the resource store, the
//! graph builder/engine, and the correlation registry. The module layout is
//! frozen at the foundation milestone; later tasks replace stub bodies without
//! moving public items.
//!
//! Dependency direction is `knect-proto <- knect-core <- knectd/knect-tui`.
//! This crate never depends on the daemon, the TUI, or the E2E harness.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod actions;
pub mod cache;
pub mod config;
pub mod correlate;
pub mod error;
pub mod graph;
pub mod kube;
pub mod links;
pub mod mutations;
pub mod operators;
pub mod policy;
pub mod storage;
pub mod store;
pub mod tap;

pub use actions::{ActionExecutor, DisabledActions};
pub use cache::{NoopCache, SnapshotCache};
pub use config::{Config, ConfigOverrides};
pub use error::CoreError;
pub use graph::{GraphBuilder, GraphEngine};
pub use kube::{ClusterReader, DiscoverySnapshot, DryRunResult, KubeReader, LogOptions};
pub use store::{ResourceStore, StoreEvent};
pub use tap::{NoopTapManager, TapManager};

/// Returns the version of the `knect-core` crate compiled into the binary.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
