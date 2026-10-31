//! Versioned wire contract for the knect daemon and its clients.
//!
//! `knect-proto` is intentionally pure: it contains no Kubernetes client,
//! runtime, filesystem, or network code. Every type is serializable and
//! schema-described so the Rust daemon, TypeScript WebUI, Lua Neovim plugin,
//! and TUI consume exactly the same contract.
//!
//! The contract is frozen at [`SCHEMA_VERSION`]. Additive optional fields are
//! allowed; existing field names, enum tags, method names, and event names are
//! not.
//!
//! # Determinism
//!
//! Maps are [`std::collections::BTreeMap`] and graph lists are ordered by the
//! producer, so serialized output is deterministic for a given topology.
//!
//! # Secrets
//!
//! Protocol types contain only redacted values. Raw secret material is held in
//! [`redaction::SecretValue`], which implements neither `Serialize` nor
//! `Deserialize` and redacts its `Debug`/`Display` output.

#![forbid(unsafe_code)]

pub mod error;
pub mod events;
pub mod graph;
pub mod health;
pub mod ids;
pub mod links;
pub mod mutation;
pub mod policy;
pub mod redaction;
pub mod rpc;
pub mod storage;
pub mod tap;

/// Compatibility re-exports for modules that call the inspector payload
/// `details`.
pub mod details {
    pub use crate::graph::{MetricsSummary, NodeDetails};
    pub use crate::health::{HealthSummary, ScaleState};
}

pub use error::{ErrorData, RpcError};
pub use events::{SseEvent, SseEventKind};
pub use graph::{Edge, NodeCategory, NodeDetails, ResourceNode, Topology};
pub use ids::{FieldPath, Gvk, ResourceId};
pub use mutation::{DiffLine, MutationDiff, MutationRecord};
pub use rpc::{RpcId, RpcRequest, RpcResponse};

/// Protocol schema version carried by every topology and checked by clients.
pub const SCHEMA_VERSION: u32 = 1;
