#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use knect_proto::ResourceId;
use knect_proto::graph::{
    Edge, EdgeKind, Health, NodeCategory, ResourceNode, Topology, TopologyStats,
};
use knect_tui::app::state::AppState;
use knect_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn node(index: usize) -> ResourceNode {
    ResourceNode {
        id: ResourceId::namespaced("bench.test/v1", "Node", "bench", format!("node-{index:04}")),
        category: if index.is_multiple_of(3) {
            NodeCategory::KService
        } else {
            NodeCategory::Revision
        },
        name: format!("node-{index:04}"),
        namespace: Some("bench".to_owned()),
        health: Health::Healthy,
        summary: String::new(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
        badges: Vec::new(),
        uid: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    }
}

fn benchmark_topology() -> Topology {
    let nodes: Vec<ResourceNode> = (0..2_000).map(node).collect();
    let mut edges = Vec::with_capacity(5_000);
    for index in 0..5_000 {
        let from = index % nodes.len();
        let mut to = (index * 7 + 1) % nodes.len();
        if to == from {
            to = (to + 1) % nodes.len();
        }
        edges.push(Edge {
            id: format!("edge-{index:05}"),
            from: nodes[from].id.clone(),
            to: nodes[to].id.clone(),
            kind: EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        });
    }
    Topology {
        schema_version: 1,
        context: "bench".to_owned(),
        generation: 1,
        stats: TopologyStats {
            node_count: nodes.len() as u64,
            edge_count: edges.len() as u64,
            ..TopologyStats::default()
        },
        nodes,
        edges,
        warnings: Vec::new(),
    }
}

fn percentile(samples: &mut [Duration], fraction: f64) -> Duration {
    samples.sort();
    let index = ((samples.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[index]
}

#[test]
#[ignore = "perf gate; run with cargo test -p knect-tui --release -- --ignored frame_budget"]
fn frame_budget_2k_nodes_5k_edges() {
    let mut state = AppState::new();
    let topology = benchmark_topology();
    let layout_started = Instant::now();
    state.set_topology(topology);
    let layout_time = layout_started.elapsed();

    let backend = TestBackend::new(160, 50);
    let mut terminal = Terminal::new(backend).expect("terminal");
    for _ in 0..3 {
        terminal
            .draw(|frame| ui::render(frame, &state))
            .expect("warmup draw");
    }

    let mut samples = Vec::with_capacity(30);
    for _ in 0..30 {
        let started = Instant::now();
        terminal
            .draw(|frame| ui::render(frame, &state))
            .expect("draw");
        samples.push(started.elapsed());
    }
    let p95 = percentile(&mut samples, 0.95);
    eprintln!(
        "frame_budget: p95={:?} layout={:?} nodes=2000 edges=5000",
        p95, layout_time
    );
    assert!(
        p95 < Duration::from_micros(16_600),
        "p95 frame budget exceeded: {p95:?}"
    );
}
