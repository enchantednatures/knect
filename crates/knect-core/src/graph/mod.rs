//! Graph builder, engine, deterministic statistics, and topology diffing.
//!
//! [`GraphBuilder`] is the mutable assembly surface correlators receive.
//! [`GraphEngine`] owns the stable `petgraph` representation and projects a
//! deterministic [`knect_proto::graph::Topology`]. External identity is always
//! [`knect_proto::ResourceId`]; `NodeIndex` values never leave the engine except
//! through explicit accessors used by graph-specific tasks.
//!
//! The public surface is re-exported here so later tasks keep importing
//! `knect_core::graph::{GraphBuilder, GraphEngine, ...}` while the
//! implementation lives in focused submodules.

pub mod engine;
pub mod patch;
pub mod stats;
pub mod support;

pub use engine::{GraphBuilder, GraphEngine};
pub use patch::diff_topologies;
pub use stats::{stats_for, stats_for_truncated};
pub use support::{edge_id, edge_id_with_suffix, edge_kind_key};
