//! Snapshot cache boundary.
//!
//! Task 5 replaces [`NoopCache`] with the redb-backed implementation. The trait
//! is synchronous because redb writes are synchronous and callers run them on
//! a blocking-safe path.

use knect_proto::Topology;

use crate::error::CoreError;

/// Persists and restores per-context topology snapshots.
pub trait SnapshotCache: Send + Sync {
    /// Loads the most recent snapshot for a context.
    fn load(&self, context: &str) -> Result<Option<Topology>, CoreError>;

    /// Saves a snapshot for a context.
    fn save(&self, context: &str, topology: &Topology) -> Result<(), CoreError>;

    /// Flushes buffered writes.
    fn flush(&self) -> Result<(), CoreError>;

    /// Clears the snapshot for a context.
    fn clear(&self, context: &str) -> Result<(), CoreError>;
}

/// Cache implementation that stores nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopCache;

impl SnapshotCache for NoopCache {
    fn load(&self, _context: &str) -> Result<Option<Topology>, CoreError> {
        Ok(None)
    }

    fn save(&self, _context: &str, _topology: &Topology) -> Result<(), CoreError> {
        Ok(())
    }

    fn flush(&self) -> Result<(), CoreError> {
        Ok(())
    }

    fn clear(&self, _context: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::graph::TopologyStats;

    fn topology() -> Topology {
        Topology {
            schema_version: knect_proto::SCHEMA_VERSION,
            context: "fixture".to_owned(),
            generation: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            warnings: Vec::new(),
            stats: TopologyStats::default(),
        }
    }

    #[test]
    fn noop_cache_round_trips_to_none() {
        let cache = NoopCache;
        assert!(cache.load("fixture").expect("load").is_none());
        cache.save("fixture", &topology()).expect("save");
        cache.flush().expect("flush");
        cache.clear("fixture").expect("clear");
    }

    #[test]
    fn cache_trait_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn SnapshotCache>();
        let cache: std::sync::Arc<dyn SnapshotCache> = std::sync::Arc::new(NoopCache);
        assert!(cache.load("x").is_ok());
    }
}
