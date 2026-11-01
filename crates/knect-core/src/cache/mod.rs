//! Snapshot cache boundary and the redb-backed implementation.
//!
//! The trait is synchronous because redb writes are synchronous and callers run
//! them on a blocking-safe path. [`RedbCache`] stores one msgpack-encoded
//! [`Topology`] per Kubernetes context, guarded by an exclusive `fs2` writer
//! lock so two daemons cannot corrupt the same cache.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use knect_proto::{SCHEMA_VERSION, Topology};
use redb::{Database, ReadableDatabase, TableDefinition};

use crate::error::CoreError;

/// Metadata table: context-independent cache state such as the schema version.
const META: TableDefinition<&str, u32> = TableDefinition::new("meta");
/// Snapshot table: context name to msgpack-encoded topology.
const SNAPSHOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("snapshots");
/// Metadata key holding the cache schema version.
const SCHEMA_VERSION_KEY: &str = "schema_version";
/// Database file name inside the cache directory.
pub const DATABASE_FILE: &str = "state.db";
/// Writer-lock file name inside the cache directory.
pub const LOCK_FILE: &str = "state.db.lock";

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

/// redb-backed snapshot cache with a single-writer lock.
pub struct RedbCache {
    database: Database,
    write_guard: Mutex<()>,
    max_snapshot_bytes: usize,
    directory: PathBuf,
    // Keeping the lock file open holds the `fs2` advisory lock for the lifetime
    // of the cache; dropping the file releases it.
    _lock: File,
}

impl std::fmt::Debug for RedbCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedbCache")
            .field("directory", &self.directory)
            .field("max_snapshot_bytes", &self.max_snapshot_bytes)
            .finish_non_exhaustive()
    }
}

impl RedbCache {
    /// Opens (or creates) a cache rooted at `path`.
    ///
    /// The directory contains `state.db` and `state.db.lock`. A second process
    /// (or a second handle in this process) attempting to open the same
    /// directory fails with [`CoreError::Cache`] while the first handle lives.
    pub fn open(path: &Path, max_snapshot_bytes: usize) -> Result<Self, CoreError> {
        std::fs::create_dir_all(path).map_err(|error| {
            CoreError::Cache(format!(
                "cannot create cache directory {}: {error}",
                path.display()
            ))
        })?;

        let lock_path = path.join(LOCK_FILE);
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| {
                CoreError::Cache(format!(
                    "cannot open cache lock {}: {error}",
                    lock_path.display()
                ))
            })?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            CoreError::Cache(format!(
                "cache {} is locked by another writer: {error}",
                path.display()
            ))
        })?;

        let database_path = path.join(DATABASE_FILE);
        let database = Database::create(&database_path).map_err(|error| {
            CoreError::Cache(format!(
                "cannot open cache database {}: {error}",
                database_path.display()
            ))
        })?;

        Ok(Self {
            database,
            write_guard: Mutex::new(()),
            max_snapshot_bytes,
            directory: path.to_path_buf(),
            _lock: lock,
        })
    }

    /// Returns the cache directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the maximum accepted serialized snapshot size.
    #[must_use]
    pub fn max_snapshot_bytes(&self) -> usize {
        self.max_snapshot_bytes
    }

    fn write_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        match self.write_guard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl SnapshotCache for RedbCache {
    fn load(&self, context: &str) -> Result<Option<Topology>, CoreError> {
        let transaction = self.database.begin_read().map_err(|error| {
            CoreError::Cache(format!("cannot begin cache read transaction: {error}"))
        })?;

        let meta = match transaction.open_table(META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => {
                return Err(CoreError::Cache(format!(
                    "cannot open cache metadata table: {error}"
                )));
            }
        };
        let stored_version = meta
            .get(SCHEMA_VERSION_KEY)
            .map_err(|error| CoreError::Cache(format!("cannot read cache schema: {error}")))?
            .map(|value| value.value());
        drop(meta);

        match stored_version {
            None => {
                return Err(CoreError::Cache(
                    "cache metadata is missing schema_version".to_owned(),
                ));
            }
            Some(version) if version != SCHEMA_VERSION => {
                return Err(CoreError::Cache(format!(
                    "cache schema version {version} does not match supported version {SCHEMA_VERSION}"
                )));
            }
            Some(_) => {}
        }

        let snapshots = transaction.open_table(SNAPSHOTS).map_err(|error| {
            CoreError::Cache(format!("cannot open cache snapshot table: {error}"))
        })?;
        let Some(value) = snapshots
            .get(context)
            .map_err(|error| CoreError::Cache(format!("cannot read cache snapshot: {error}")))?
        else {
            return Ok(None);
        };
        // Deserialize directly from the redb page-backed bytes. Copying the
        // snapshot first doubles the cold-read cost for a multi-megabyte graph.
        rmp_serde::from_slice(value.value())
            .map(Some)
            .map_err(|error| {
                CoreError::Cache(format!(
                    "cache snapshot for context '{context}' is corrupt: {error}"
                ))
            })
    }

    fn save(&self, context: &str, topology: &Topology) -> Result<(), CoreError> {
        let _guard = self.write_guard();
        let bytes = rmp_serde::to_vec_named(topology)
            .map_err(|error| CoreError::Cache(format!("cannot encode cache snapshot: {error}")))?;
        if bytes.len() > self.max_snapshot_bytes {
            return Err(CoreError::Cache(format!(
                "snapshot for context '{context}' is {} bytes, exceeding the {} byte limit",
                bytes.len(),
                self.max_snapshot_bytes
            )));
        }

        let transaction = self.database.begin_write().map_err(|error| {
            CoreError::Cache(format!("cannot begin cache write transaction: {error}"))
        })?;
        {
            let mut meta = transaction.open_table(META).map_err(|error| {
                CoreError::Cache(format!("cannot open cache metadata table: {error}"))
            })?;
            meta.insert(SCHEMA_VERSION_KEY, SCHEMA_VERSION)
                .map_err(|error| {
                    CoreError::Cache(format!("cannot write cache schema version: {error}"))
                })?;
        }
        {
            let mut snapshots = transaction.open_table(SNAPSHOTS).map_err(|error| {
                CoreError::Cache(format!("cannot open cache snapshot table: {error}"))
            })?;
            snapshots
                .insert(context, bytes.as_slice())
                .map_err(|error| {
                    CoreError::Cache(format!("cannot write cache snapshot: {error}"))
                })?;
        }
        transaction
            .commit()
            .map_err(|error| CoreError::Cache(format!("cannot commit cache snapshot: {error}")))
    }

    fn flush(&self) -> Result<(), CoreError> {
        // Every save is committed durably in its own transaction; there is no
        // buffered state to flush.
        Ok(())
    }

    fn clear(&self, context: &str) -> Result<(), CoreError> {
        let _guard = self.write_guard();
        let transaction = self.database.begin_write().map_err(|error| {
            CoreError::Cache(format!("cannot begin cache write transaction: {error}"))
        })?;
        {
            let mut snapshots = transaction.open_table(SNAPSHOTS).map_err(|error| {
                CoreError::Cache(format!("cannot open cache snapshot table: {error}"))
            })?;
            snapshots.remove(context).map_err(|error| {
                CoreError::Cache(format!("cannot remove cache snapshot: {error}"))
            })?;
        }
        transaction
            .commit()
            .map_err(|error| CoreError::Cache(format!("cannot commit cache clear: {error}")))
    }
}

/// Converts a `u64` byte limit into a `usize`, saturating on narrower targets.
#[must_use]
pub fn snapshot_limit_bytes(limit: u64) -> usize {
    usize::try_from(limit).unwrap_or(usize::MAX)
}

/// Convenience wrapper so callers can construct a cache without juggling the
/// lock file; errors are already typed by [`RedbCache::open`].
pub fn open_redb_cache(path: &Path, max_snapshot_bytes: u64) -> Result<RedbCache, CoreError> {
    RedbCache::open(path, snapshot_limit_bytes(max_snapshot_bytes))
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

    #[test]
    fn redb_cache_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RedbCache>();
    }

    #[test]
    fn redb_cache_round_trips_and_isolates_contexts() {
        let directory = tempfile::tempdir().expect("temp dir");
        let cache = RedbCache::open(directory.path(), 64 * 1024 * 1024).expect("open");
        assert!(cache.load("missing").expect("load missing").is_none());
        cache.save("fixture", &topology()).expect("save");
        assert_eq!(
            cache.load("fixture").expect("load").map(|t| t.context),
            Some("fixture".to_owned())
        );
        assert!(cache.load("other").expect("load other").is_none());
        cache.clear("fixture").expect("clear");
        assert!(cache.load("fixture").expect("load after clear").is_none());
        cache.flush().expect("flush");
    }

    #[test]
    fn second_writer_is_rejected() {
        let directory = tempfile::tempdir().expect("temp dir");
        let first = RedbCache::open(directory.path(), 1_024).expect("first open");
        let error = RedbCache::open(directory.path(), 1_024).expect_err("second open must fail");
        assert!(matches!(error, CoreError::Cache(_)));
        drop(first);
        RedbCache::open(directory.path(), 1_024).expect("open after drop");
    }
}
