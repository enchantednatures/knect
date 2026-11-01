//! Storage tracing boundary.
//!
//! Resolves the persistent-volume chain behind Knative workloads
//! (`Revision`/`Deployment`/`Pod` -> PVC -> PV -> StorageClass -> Rook Ceph)
//! and lints ReadWriteOnce claims mounted by autoscaled services.

mod backend;
mod catalog;
mod lint;
mod mounts;
mod owners;
mod summary;
mod trace;

pub use knect_proto::storage::{AccessMode, StorageConflictWarning, StorageRecord, StorageWarning};

pub(crate) use lint::{CONFLICT_BADGE, CONFLICT_CODE, conflict_reason, conflicts};
pub(crate) use summary::{
    backend_health, backend_summary, claim_health, claim_summary, storage_class_summary,
    volume_health, volume_summary,
};
pub(crate) use trace::{StorageTrace, trace};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;

/// Parses a Kubernetes persistent-volume access mode string.
///
/// Unknown values are preserved as [`AccessMode::Unknown`] so callers can keep
/// tracing the rest of a claim instead of dropping it.
#[must_use]
pub fn parse_access_mode(value: &str) -> AccessMode {
    match value {
        "ReadWriteOnce" => AccessMode::ReadWriteOnce,
        "ReadOnlyMany" => AccessMode::ReadOnlyMany,
        "ReadWriteMany" => AccessMode::ReadWriteMany,
        "ReadWriteOncePod" => AccessMode::ReadWriteOncePod,
        _ => AccessMode::Unknown,
    }
}

/// Parses and deduplicates access modes while preserving their declared order.
#[must_use]
pub fn parse_access_modes<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<AccessMode> {
    let mut modes = Vec::new();
    for value in values {
        let mode = parse_access_mode(value);
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes
}

/// Returns true for access modes that only permit a single writer.
#[must_use]
pub const fn is_read_write_once(mode: AccessMode) -> bool {
    matches!(
        mode,
        AccessMode::ReadWriteOnce | AccessMode::ReadWriteOncePod
    )
}

/// Stable display string for an access mode.
#[must_use]
pub const fn access_mode_label(mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::ReadWriteOnce => "ReadWriteOnce",
        AccessMode::ReadOnlyMany => "ReadOnlyMany",
        AccessMode::ReadWriteMany => "ReadWriteMany",
        AccessMode::ReadWriteOncePod => "ReadWriteOncePod",
        AccessMode::Unknown => "Unknown",
    }
}

/// Resolves storage records for the current store.
pub trait StorageResolver: Send + Sync {
    /// Returns storage relationships discovered in the store.
    fn records(&self, ctx: &CorrelationContext) -> Result<Vec<StorageRecord>, CoreError>;
}

/// Default resolver backed by the sanitized resource store.
#[derive(Debug, Default, Clone, Copy)]
pub struct StoreStorageResolver;

impl StorageResolver for StoreStorageResolver {
    fn records(&self, ctx: &CorrelationContext) -> Result<Vec<StorageRecord>, CoreError> {
        Ok(trace(ctx).records)
    }
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(
    resolver: std::sync::Arc<dyn StorageResolver>,
) -> std::sync::Arc<dyn StorageResolver> {
    resolver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_modes_parse_and_deduplicate_in_order() {
        assert_eq!(
            parse_access_modes(["ReadWriteOnce", "ReadWriteMany", "ReadWriteOnce"]),
            vec![AccessMode::ReadWriteOnce, AccessMode::ReadWriteMany]
        );
        assert_eq!(
            parse_access_mode("ReadWriteOncePod"),
            AccessMode::ReadWriteOncePod
        );
        assert!(is_read_write_once(AccessMode::ReadWriteOnce));
        assert!(is_read_write_once(AccessMode::ReadWriteOncePod));
        assert!(!is_read_write_once(AccessMode::ReadWriteMany));
        assert_eq!(access_mode_label(AccessMode::ReadOnlyMany), "ReadOnlyMany");
        assert_eq!(parse_access_mode("SomethingElse"), AccessMode::Unknown);
    }
}
