//! Shared daemon state.
//!
//! Topology reads clone an `Arc` from a read-mostly lock. `arc-swap` is not in
//! the pinned workspace dependency set, so the foundation uses
//! `std::sync::RwLock<Arc<Topology>>`: readers hold the lock only long enough
//! to clone the pointer, and writers replace the pointer. Task 5 may swap the
//! cell for `ArcSwap` without changing the accessor API.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use knect_core::config::Config;
use knect_core::store::ResourceStore;
use knect_core::{
    ActionExecutor, DisabledActions, NoopCache, NoopTapManager, SnapshotCache, TapManager,
};
use knect_proto::events::{Heartbeat, SseEvent, SseEventKind};
use knect_proto::graph::Topology;
use knect_proto::rpc::WatcherStatus;
use tokio::sync::RwLock as AsyncRwLock;

use crate::transport::sse::EventHub;

/// Shared state passed to every RPC method, HTTP handler, and runtime task.
#[derive(Clone)]
pub struct AppState {
    /// Effective configuration.
    pub config: Arc<Config>,
    /// Current topology. Reads clone the inner `Arc` under a short lock.
    pub topology: Arc<RwLock<Arc<Topology>>>,
    /// Sanitized resource store.
    pub store: Arc<AsyncRwLock<ResourceStore>>,
    /// Snapshot cache. Foundation implementation is [`NoopCache`].
    pub cache: Arc<dyn SnapshotCache>,
    /// Guarded action executor. Foundation implementation is [`DisabledActions`].
    pub actions: Arc<dyn ActionExecutor>,
    /// Tap manager. Foundation implementation is [`NoopTapManager`].
    pub tap: Arc<dyn TapManager>,
    /// Monotonic topology generation.
    pub generation: Arc<AtomicU64>,
    /// Process start instant for uptime.
    pub started_at: Instant,
    /// Wall-clock process start time.
    pub started_at_utc: DateTime<Utc>,
    /// SSE broadcast ring and sender.
    pub events: Arc<EventHub>,
    /// Watcher health. Empty until Task 3.
    pub watchers: Arc<RwLock<Vec<WatcherStatus>>>,
    /// Last discovery resync time. Set by fixture startup; Task 3 updates it.
    pub last_resync: Arc<RwLock<Option<DateTime<Utc>>>>,
    /// True when the daemon runs from fixtures and never contacts Kubernetes.
    pub fixture: bool,
    /// Per-client heartbeat cadence.
    pub heartbeat_interval: Duration,
}

impl AppState {
    /// Builds the foundation state with empty topology and stub services.
    #[must_use]
    pub fn new(mut config: Config, fixture: bool) -> Self {
        if fixture {
            config.daemon.context = "fixture".to_owned();
        }
        let context = config.daemon.context.clone();
        let topology = empty_topology(&context, 0);
        Self {
            config: Arc::new(config),
            topology: Arc::new(RwLock::new(Arc::new(topology))),
            store: Arc::new(AsyncRwLock::new(ResourceStore::new())),
            cache: Arc::new(NoopCache),
            actions: Arc::new(DisabledActions),
            tap: Arc::new(NoopTapManager),
            generation: Arc::new(AtomicU64::new(0)),
            started_at: Instant::now(),
            started_at_utc: Utc::now(),
            events: Arc::new(EventHub::new(1_000)),
            watchers: Arc::new(RwLock::new(Vec::new())),
            last_resync: Arc::new(RwLock::new(None)),
            fixture,
            heartbeat_interval: Duration::from_secs(15),
        }
    }

    /// Overrides the heartbeat cadence (tests use a short interval).
    #[must_use]
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Returns the current topology snapshot.
    #[must_use]
    pub fn topology(&self) -> Arc<Topology> {
        match self.topology.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Replaces the topology and synchronizes the generation counter.
    pub fn set_topology(&self, topology: Topology) {
        self.generation
            .store(topology.generation, Ordering::Relaxed);
        let mut guard = match self.topology.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Arc::new(topology);
    }

    /// Returns the current generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Returns the effective context name.
    #[must_use]
    pub fn context(&self) -> String {
        if self.fixture {
            "fixture".to_owned()
        } else {
            self.config.daemon.context.clone()
        }
    }

    /// True when both the master switch and the feature gate allow actions.
    #[must_use]
    pub fn actions_enabled(&self) -> bool {
        self.config.actions.enabled && self.config.daemon.allow_actions
    }

    /// Returns the last resync time.
    #[must_use]
    pub fn last_resync(&self) -> Option<DateTime<Utc>> {
        match self.last_resync.read() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Records the last resync time.
    pub fn set_last_resync(&self, value: Option<DateTime<Utc>>) {
        let mut guard = match self.last_resync.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = value;
    }

    /// Returns watcher health snapshots.
    #[must_use]
    pub fn watchers(&self) -> Vec<WatcherStatus> {
        match self.watchers.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Publishes a graph event to the SSE ring and broadcast channel.
    pub fn publish(&self, kind: SseEventKind) -> SseEvent {
        self.events.publish(kind)
    }

    /// Builds a per-client heartbeat event without broadcasting it.
    #[must_use]
    pub fn heartbeat_event(&self) -> SseEvent {
        let topology = self.topology();
        let heartbeat = Heartbeat {
            uptime_secs: self.started_at.elapsed().as_secs(),
            generation: topology.generation,
            node_count: topology.stats.node_count,
            edge_count: topology.stats.edge_count,
            ts: Utc::now(),
        };
        self.events
            .next_event(SseEventKind::Heartbeat { data: heartbeat })
    }
}

/// Builds an empty topology for a context and generation.
#[must_use]
pub fn empty_topology(context: &str, generation: u64) -> Topology {
    Topology {
        schema_version: knect_proto::SCHEMA_VERSION,
        context: context.to_owned(),
        generation,
        nodes: Vec::new(),
        edges: Vec::new(),
        warnings: Vec::new(),
        stats: knect_proto::graph::TopologyStats::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_state_starts_empty_and_named_fixture() {
        let state = AppState::new(Config::default(), true);
        let topology = state.topology();
        assert_eq!(topology.context, "fixture");
        assert_eq!(topology.generation, 0);
        assert!(topology.nodes.is_empty());
        assert!(topology.edges.is_empty());
        assert_eq!(state.generation(), 0);
        assert_eq!(state.context(), "fixture");
        assert!(!state.actions_enabled());
        assert!(state.watchers().is_empty());
        assert!(state.last_resync().is_none());
    }

    #[test]
    fn set_topology_updates_generation() {
        let state = AppState::new(Config::default(), true);
        let mut topology = empty_topology("fixture", 9);
        topology.warnings.push(knect_proto::graph::Warning {
            code: "test".to_owned(),
            message: "warning".to_owned(),
            severity: Default::default(),
            resource: None,
        });
        topology.stats.warning_count = 1;
        state.set_topology(topology);
        assert_eq!(state.generation(), 9);
        assert_eq!(state.topology().stats.warning_count, 1);
    }

    #[test]
    fn heartbeat_reflects_counts_without_broadcasting() {
        let state = AppState::new(Config::default(), true);
        let mut receiver = state.events.subscribe();
        let event = state.heartbeat_event();
        assert_eq!(event.event_type(), "daemon.heartbeat");
        assert!(matches!(event.kind, SseEventKind::Heartbeat { .. }));
        assert!(receiver.try_recv().is_err());
    }
}
