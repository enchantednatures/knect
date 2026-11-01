#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Release-mode cold-start measurement for the redb snapshot cache.
//!
//! Run with:
//! `cargo test -p knect-core cache_cold_start --release -- --nocapture`

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use knect_core::GraphBuilder;
use knect_core::cache::{RedbCache, SnapshotCache};
use knect_proto::graph::{Edge, EdgeKind, Health};
use knect_proto::{NodeCategory, ResourceId, ResourceNode, Topology};

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

#[test]
fn cache_cold_start_2k_nodes_5k_edges() {
    let directory = tempfile::tempdir().expect("temp dir");
    let topology = big_topology(2_000, 5_000);
    {
        let cache = RedbCache::open(directory.path(), 128 * 1024 * 1024).expect("open writer");
        cache.save("kind-knect-e2e", &topology).expect("save");
    }

    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(15)
    };
    // Timing assertions on a shared CI host are scheduler-sensitive. Take
    // three independent cold loads (fresh cache handle each time) and assert
    // the best sample fits the budget; an algorithmic regression moves every
    // sample over the line. All samples are printed for the evidence record.
    let mut samples = Vec::new();
    let mut encoded_bytes = 0;
    for pass in 0..3 {
        let cache = RedbCache::open(directory.path(), 128 * 1024 * 1024).expect("open reader");
        let started = Instant::now();
        let loaded = cache
            .load("kind-knect-e2e")
            .expect("load")
            .expect("snapshot");
        let elapsed = started.elapsed();
        assert_eq!(loaded.nodes.len(), 2_000);
        assert_eq!(loaded.edges.len(), 5_000);
        encoded_bytes = rmp_serde::to_vec_named(&loaded)
            .expect("measure encoded size")
            .len();
        eprintln!("cache_cold_start pass {pass}: {elapsed:?}");
        samples.push(elapsed);
    }
    samples.sort();
    let best = samples[0];
    let median = samples[1];
    eprintln!(
        "cache_cold_start: 2000 nodes / 5000 edges best={best:?} median={median:?} samples={samples:?} ({} bytes msgpack)",
        encoded_bytes
    );
    assert!(
        best < budget,
        "cold load best sample took {best:?}, budget {budget:?} (samples {samples:?})"
    );
}
