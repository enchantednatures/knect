#![allow(clippy::unwrap_used, clippy::expect_used)]

//! redb cache behavior on the public `SnapshotCache` surface.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use knect_core::cache::{DATABASE_FILE, RedbCache, SnapshotCache};
use knect_core::{CoreError, GraphBuilder};
use knect_proto::graph::{Edge, EdgeKind, Health};
use knect_proto::{NodeCategory, ResourceId, ResourceNode, SCHEMA_VERSION, Topology};

fn node(index: usize) -> ResourceNode {
    ResourceNode {
        id: ResourceId::namespaced(
            "apps/v1",
            "Deployment",
            "default",
            format!("app-{index:05}"),
        ),
        category: NodeCategory::Deployment,
        name: format!("app-{index:05}"),
        namespace: Some("default".to_owned()),
        health: Health::Healthy,
        summary: "ok".to_owned(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
        badges: Vec::new(),
        uid: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    }
}

fn big_topology(nodes: usize, edges: usize) -> Topology {
    let mut builder = GraphBuilder::new("kind-knect-e2e").with_generation(42);
    let built: Vec<ResourceNode> = (0..nodes).map(node).collect();
    for item in &built {
        builder.add_node(item.clone());
    }
    for index in 0..edges {
        let from = &built[index % built.len()];
        let to = &built[(index + 1) % built.len()];
        builder.add_edge(Edge {
            id: format!("edge-{index:05}"),
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        });
    }
    builder.build()
}

fn small_topology(name: &str) -> Topology {
    let mut builder = GraphBuilder::new(name).with_generation(1);
    builder.add_node(node(1));
    builder.build()
}

fn open(path: &std::path::Path, limit: usize) -> RedbCache {
    RedbCache::open(path, limit).expect("cache opens")
}

#[test]
fn round_trip_empty_topology() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = open(directory.path(), 64 * 1024 * 1024);
    let topology = small_topology("fixture");
    cache.save("fixture", &topology).expect("save");
    let loaded = cache
        .load("fixture")
        .expect("load")
        .expect("snapshot exists");
    assert_eq!(loaded, topology);
    assert_eq!(loaded.schema_version, SCHEMA_VERSION);
}

#[test]
fn round_trip_large_topology() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = open(directory.path(), 128 * 1024 * 1024);
    let topology = big_topology(2_000, 5_000);
    cache.save("large", &topology).expect("save");
    let loaded = cache.load("large").expect("load").expect("snapshot");
    assert_eq!(loaded.nodes.len(), 2_000);
    assert_eq!(loaded.edges.len(), 5_000);
    assert_eq!(loaded.stats.node_count, 2_000);
    assert_eq!(loaded.stats.edge_count, 5_000);
}

#[test]
fn missing_context_returns_none() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = open(directory.path(), 1_024);
    assert!(cache.load("absent").expect("load").is_none());
    cache
        .save("fixture", &small_topology("fixture"))
        .expect("save");
    assert!(cache.load("absent").expect("load").is_none());
}

#[test]
fn corrupt_snapshot_bytes_are_a_typed_cache_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    {
        let cache = open(directory.path(), 1_024);
        cache
            .save("fixture", &small_topology("fixture"))
            .expect("save");
    }

    let database = redb::Database::create(directory.path().join(DATABASE_FILE)).expect("reopen");
    let transaction = database.begin_write().expect("write txn");
    {
        const SNAPSHOTS: redb::TableDefinition<&str, &[u8]> =
            redb::TableDefinition::new("snapshots");
        let mut table = transaction.open_table(SNAPSHOTS).expect("snapshots");
        table
            .insert("fixture", b"this is not msgpack".as_slice())
            .expect("corrupt write");
    }
    transaction.commit().expect("commit");
    drop(database);

    let cache = open(directory.path(), 1_024);
    let error = cache.load("fixture").expect_err("corrupt cache must fail");
    assert!(matches!(error, CoreError::Cache(_)));
    assert!(error.to_string().contains("corrupt"));
}

#[test]
fn schema_mismatch_is_a_typed_cache_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    {
        let cache = open(directory.path(), 1_024);
        cache
            .save("fixture", &small_topology("fixture"))
            .expect("save");
    }

    let database = redb::Database::create(directory.path().join(DATABASE_FILE)).expect("reopen");
    let transaction = database.begin_write().expect("write txn");
    {
        let mut meta = transaction
            .open_table(redb::TableDefinition::<&str, u32>::new("meta"))
            .expect("meta");
        meta.insert("schema_version", 999_u32)
            .expect("schema write");
    }
    transaction.commit().expect("commit");
    drop(database);

    let cache = open(directory.path(), 1_024);
    let error = cache
        .load("fixture")
        .expect_err("schema mismatch must fail");
    assert!(matches!(error, CoreError::Cache(_)));
    assert!(error.to_string().contains("schema version"));
}

#[test]
fn corrupt_database_file_is_reported_at_open() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join(DATABASE_FILE), b"not a redb database")
        .expect("write garbage database");
    let error = RedbCache::open(directory.path(), 1_024).expect_err("corrupt database must fail");
    assert!(matches!(error, CoreError::Cache(_)));
}

#[test]
fn lock_conflict_rejects_a_second_writer() {
    let directory = tempfile::tempdir().expect("temp dir");
    let first = open(directory.path(), 1_024);
    let error = RedbCache::open(directory.path(), 1_024).expect_err("second writer must fail");
    assert!(matches!(error, CoreError::Cache(_)));
    assert!(error.to_string().contains("locked"));
    drop(first);
    open(directory.path(), 1_024);
}

#[test]
fn max_snapshot_size_is_enforced() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = open(directory.path(), 128);
    let topology = big_topology(200, 400);
    let error = cache
        .save("large", &topology)
        .expect_err("oversize must fail");
    assert!(matches!(error, CoreError::Cache(_)));
    assert!(error.to_string().contains("exceeding"));
    assert!(cache.load("large").expect("load").is_none());
}

#[test]
fn context_isolation_keeps_snapshots_separate() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = open(directory.path(), 64 * 1024 * 1024);
    let first = small_topology("first");
    let second = small_topology("second");
    cache.save("first", &first).expect("save first");
    cache.save("second", &second).expect("save second");
    assert_eq!(cache.load("first").expect("load first"), Some(first));
    assert_eq!(cache.load("second").expect("load second"), Some(second));

    cache.clear("first").expect("clear first");
    assert!(cache.load("first").expect("load first").is_none());
    assert!(cache.load("second").expect("load second").is_some());
}

#[test]
fn cold_load_of_large_snapshot_is_fast() {
    let directory = tempfile::tempdir().expect("temp dir");
    let topology = big_topology(2_000, 5_000);
    {
        let cache = open(directory.path(), 128 * 1024 * 1024);
        cache.save("large", &topology).expect("save");
    }
    let cache = open(directory.path(), 128 * 1024 * 1024);
    let started = Instant::now();
    let loaded = cache.load("large").expect("load").expect("snapshot");
    let elapsed = started.elapsed();
    eprintln!("cache cold load 2k/5k: {elapsed:?}");
    assert_eq!(loaded.nodes.len(), 2_000);
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(15)
    };
    assert!(
        elapsed < budget,
        "cold load took {elapsed:?}, budget {budget:?}"
    );
}
