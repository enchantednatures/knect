//! Live daemon runtime: discovery, watchers, sanitization, recompute, cache,
//! and graph-patch publication.
//!
//! Fixture mode loads the fixture directory, runs the correlation pipeline once,
//! and never constructs a Kubernetes client or touches the snapshot cache.
//! Normal mode discovers resources, supervises one watcher per GVK, debounces
//! recomputes, and persists snapshots on a blocking-safe path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use knect_core::correlate::{CorrelationContext, CorrelationPipeline};
use knect_core::graph::{GraphBuilder, GraphEngine};
use knect_core::kube::{
    ClusterReader, DiscoveredResource, DiscoveryManager, DiscoverySnapshot, KubeReader, Sanitizer,
    WatchHandle, WatchManager, WatchUpdate,
};
use knect_core::store::{ResourceStore, resource_id};
use knect_core::{CoreError, SnapshotCache};
use knect_proto::Gvk;
use knect_proto::rpc::WatcherStatus;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::app::empty_topology;
use crate::error::DaemonError;
use crate::fixture::{self, FixtureMode};

const UPDATE_QUEUE: usize = 2_048;
const SHUTDOWN_WATCHER_TIMEOUT: Duration = Duration::from_secs(2);
const IDLE_DEBOUNCE: Duration = Duration::from_secs(86_400);

#[derive(Debug, Clone)]
struct WatcherHealth {
    healthy: bool,
    last_event: Option<DateTime<Utc>>,
}

impl WatcherHealth {
    fn new() -> Self {
        Self {
            healthy: true,
            last_event: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct IngestOutcome {
    changed: bool,
    needs_rediscovery: bool,
}

/// One daemon runtime instance.
pub struct Runtime {
    state: AppState,
    fixture: FixtureMode,
    cancel: CancellationToken,
    watch_cancel: CancellationToken,
    kubeconfig: Option<PathBuf>,
    reader_override: Option<Arc<dyn ClusterReader>>,
    watchers: Mutex<Vec<WatchHandle>>,
    watch_manager: Mutex<Option<WatchManager>>,
    discovery: Mutex<Option<DiscoveryManager>>,
    updates: Mutex<Option<(mpsc::Sender<WatchUpdate>, mpsc::Receiver<WatchUpdate>)>>,
    watched: Mutex<BTreeMap<Gvk, WatcherHealth>>,
    sanitizer: Sanitizer,
    pipeline: CorrelationPipeline,
    engine: Mutex<GraphEngine>,
}

impl Runtime {
    /// Creates a runtime. `start` must run before `run`.
    #[must_use]
    pub fn new(state: AppState, fixture: FixtureMode, cancel: CancellationToken) -> Self {
        let sanitizer = Sanitizer::from_watch_config(&state.config.watch);
        let engine = GraphEngine::new(state.context());
        Self {
            state,
            fixture,
            cancel,
            watch_cancel: CancellationToken::new(),
            kubeconfig: None,
            reader_override: None,
            watchers: Mutex::new(Vec::new()),
            watch_manager: Mutex::new(None),
            discovery: Mutex::new(None),
            updates: Mutex::new(None),
            watched: Mutex::new(BTreeMap::new()),
            sanitizer,
            pipeline: CorrelationPipeline::default(),
            engine: Mutex::new(engine),
        }
    }

    /// Supplies an explicit kubeconfig path for normal mode.
    #[must_use]
    pub fn with_kubeconfig(mut self, kubeconfig: Option<PathBuf>) -> Self {
        self.kubeconfig = kubeconfig;
        self
    }

    /// Injects a reader for tests. Production construction uses [`KubeReader`].
    #[must_use]
    pub fn with_cluster_reader(mut self, reader: Arc<dyn ClusterReader>) -> Self {
        self.reader_override = Some(reader);
        self
    }

    /// Performs startup work and installs the initial topology.
    pub async fn start(&self) -> Result<(), DaemonError> {
        if self.fixture.is_enabled() {
            let outcome = fixture::initialize(&self.state, &self.fixture).await?;
            self.sanitize_fixture_store().await;
            self.recompute(false).await?;
            let topology = self.state.topology();
            tracing::info!(
                context = %outcome.context,
                objects = outcome.object_count,
                nodes = topology.nodes.len(),
                edges = topology.edges.len(),
                "fixture mode started; no Kubernetes client was constructed"
            );
            return Ok(());
        }

        let mut loaded = false;
        if self.state.config.cache.enabled {
            match self.state.cache.load(&self.state.context()) {
                Ok(Some(topology)) => {
                    tracing::info!(
                        generation = topology.generation,
                        nodes = topology.nodes.len(),
                        "loaded cached topology snapshot"
                    );
                    self.state.set_topology(topology);
                    loaded = true;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "snapshot cache load failed; starting from discovery");
                }
            }
        }
        if !loaded {
            let generation = self.state.generation();
            self.state
                .set_topology(empty_topology(&self.state.context(), generation));
        }

        let reader = match self.build_reader().await {
            Ok(reader) => reader,
            Err(error) => {
                tracing::warn!(%error, "cannot construct the Kubernetes reader; serving cached state");
                return Ok(());
            }
        };

        let watch_manager = WatchManager::new(Arc::clone(&reader), self.state.config.watch.clone());
        let (sender, receiver) = mpsc::channel(UPDATE_QUEUE);
        {
            let mut updates = lock(&self.updates);
            *updates = Some((sender.clone(), receiver));
        }
        {
            let mut manager = lock(&self.watch_manager);
            *manager = Some(watch_manager.clone());
        }

        let discovery = DiscoveryManager::new(reader, self.state.config.discovery.clone());
        {
            let mut stored = lock(&self.discovery);
            *stored = Some(discovery.clone());
        }
        match discovery.discover().await {
            Ok(snapshot) => {
                self.install_discovery(&watch_manager, &snapshot, &sender)
                    .await?;
            }
            Err(error) => {
                tracing::warn!(%error, "initial discovery failed; will retry periodically");
            }
        }
        Ok(())
    }

    async fn build_reader(&self) -> Result<Arc<dyn ClusterReader>, CoreError> {
        if let Some(reader) = &self.reader_override {
            return Ok(Arc::clone(reader));
        }
        if let Some(reader) = &self.state.reader {
            return Ok(Arc::clone(reader));
        }
        let context = self.state.config.daemon.context.as_str();
        let reader = KubeReader::from_context(
            (!context.is_empty()).then_some(context),
            self.kubeconfig.as_deref(),
        )
        .await?;
        Ok(Arc::new(reader))
    }

    async fn sanitize_fixture_store(&self) {
        let objects = {
            let store = self.state.store.read().await;
            store.list()
        };
        let mut rebuilt = ResourceStore::new();
        for object in objects {
            let sanitized = self.sanitizer.sanitize((*object).clone());
            let id = resource_id(&sanitized.object);
            if sanitized.secret_material.is_empty() {
                self.state.secret_material.remove(&id);
            } else {
                self.state
                    .secret_material
                    .insert(id, sanitized.secret_material);
            }
            rebuilt.upsert(sanitized.object);
        }
        *self.state.store.write().await = rebuilt;
    }

    async fn install_discovery(
        &self,
        manager: &WatchManager,
        snapshot: &DiscoverySnapshot,
        sender: &mpsc::Sender<WatchUpdate>,
    ) -> Result<(), DaemonError> {
        for warning in &snapshot.warnings {
            tracing::warn!(%warning, "discovery warning");
        }

        let present: BTreeSet<Gvk> = snapshot
            .resources
            .iter()
            .map(|resource| resource.gvk.clone())
            .collect();
        let new_resources: Vec<DiscoveredResource> = {
            let mut watched = lock(&self.watched);
            watched.retain(|gvk, _| present.contains(gvk));
            snapshot
                .resources
                .iter()
                .filter(|resource| !watched.contains_key(&resource.gvk))
                .cloned()
                .collect()
        };

        if !new_resources.is_empty() {
            let handles = manager.spawn_with_updates(
                &new_resources,
                self.watch_cancel.clone(),
                sender.clone(),
            );
            let mut watched = lock(&self.watched);
            let mut watchers = lock(&self.watchers);
            for handle in handles {
                tracing::info!(gvk = %handle.gvk, "watching resource");
                watched.insert(handle.gvk.clone(), WatcherHealth::new());
                watchers.push(handle);
            }
        }

        self.state.set_last_resync(Some(Utc::now()));
        self.refresh_watcher_status().await;
        Ok(())
    }

    async fn rediscover(&self, manager: &WatchManager, sender: &mpsc::Sender<WatchUpdate>) {
        let discovery = {
            let stored = lock(&self.discovery);
            stored.clone()
        };
        let Some(discovery) = discovery else {
            return;
        };
        match discovery.discover().await {
            Ok(snapshot) => {
                if let Err(error) = self.install_discovery(manager, &snapshot, sender).await {
                    tracing::warn!(%error, "cannot install discovered watchers");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "periodic rediscovery failed");
            }
        }
    }

    async fn refresh_watcher_status(&self) {
        let watched = lock(&self.watched).clone();
        let statuses: Vec<WatcherStatus> = {
            let store = self.state.store.read().await;
            watched
                .into_iter()
                .map(|(gvk, health)| WatcherStatus {
                    resource_count: u64::try_from(store.ids_by_gvk(&gvk).len()).unwrap_or(u64::MAX),
                    gvk,
                    healthy: health.healthy,
                    last_event: health.last_event,
                })
                .collect()
        };
        self.state.set_watchers(statuses);
    }

    fn mark_watcher(&self, gvk: &Gvk, healthy: bool) {
        let mut watched = lock(&self.watched);
        if let Some(health) = watched.get_mut(gvk) {
            health.healthy = healthy;
            health.last_event = Some(Utc::now());
        }
    }

    fn forget_watcher(&self, gvk: &Gvk) {
        lock(&self.watched).remove(gvk);
    }

    async fn ingest(&self, update: WatchUpdate) -> IngestOutcome {
        match update {
            WatchUpdate::InitApply { gvk, object, .. } | WatchUpdate::Apply { gvk, object, .. } => {
                let sanitized = self.sanitizer.sanitize(object);
                let id = resource_id(&sanitized.object);
                if sanitized.secret_material.is_empty() {
                    self.state.secret_material.remove(&id);
                } else {
                    self.state
                        .secret_material
                        .insert(id, sanitized.secret_material);
                }
                self.state.store.write().await.upsert(sanitized.object);
                self.mark_watcher(&gvk, true);
                IngestOutcome {
                    changed: true,
                    needs_rediscovery: false,
                }
            }
            WatchUpdate::Delete { gvk, id, .. } => {
                self.state.store.write().await.remove(&id);
                self.state.secret_material.remove(&id);
                self.mark_watcher(&gvk, true);
                IngestOutcome {
                    changed: true,
                    needs_rediscovery: false,
                }
            }
            WatchUpdate::Init { gvk } | WatchUpdate::InitDone { gvk } => {
                self.mark_watcher(&gvk, true);
                IngestOutcome::default()
            }
            WatchUpdate::Retry {
                gvk,
                attempt,
                delay_ms,
            } => {
                tracing::warn!(%gvk, attempt, delay_ms, "watcher retrying after failure");
                self.mark_watcher(&gvk, false);
                IngestOutcome::default()
            }
            WatchUpdate::ResourceGone { gvk, message } => {
                tracing::warn!(%gvk, %message, "watched resource disappeared; rediscovering");
                self.forget_watcher(&gvk);
                IngestOutcome {
                    changed: false,
                    needs_rediscovery: true,
                }
            }
            WatchUpdate::RetriesExhausted { gvk, attempts } => {
                tracing::warn!(%gvk, attempts, "watcher retries exhausted; rediscovering");
                self.forget_watcher(&gvk);
                IngestOutcome {
                    changed: false,
                    needs_rediscovery: true,
                }
            }
        }
    }

    async fn recompute(&self, publish: bool) -> Result<(), DaemonError> {
        let store = {
            let store = self.state.store.read().await;
            store.clone()
        };
        let generation = self.state.generation().saturating_add(1);
        let context = self.state.context();
        let ctx = CorrelationContext::new_with_secrets(
            context.clone(),
            generation,
            Arc::new(store),
            self.state.secret_material(),
        );
        let mut builder = GraphBuilder::new(&context).with_generation(generation);

        if let Err(error) = self.pipeline.run(&ctx, &mut builder) {
            tracing::warn!(%error, "correlation pipeline failed; keeping previous topology");
            return Ok(());
        }
        let topology = {
            let mut engine = lock(&self.engine);
            if let Err(error) = engine.apply(builder) {
                tracing::warn!(%error, "graph engine rejected the projection");
                return Ok(());
            }
            engine.topology()
        };

        if publish {
            self.state.set_topology_and_publish(topology.clone());
        } else {
            self.state.set_topology(topology.clone());
        }
        self.refresh_watcher_status().await;

        if self.state.config.cache.enabled && !self.fixture.is_enabled() {
            let cache = Arc::clone(&self.state.cache);
            let context = context.clone();
            let snapshot = topology;
            match tokio::task::spawn_blocking(move || cache.save(&context, &snapshot)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "snapshot cache save failed");
                }
                Err(error) => {
                    tracing::warn!(%error, "snapshot cache save task failed");
                }
            }
        }
        Ok(())
    }

    /// Runs the watcher/recompute supervisor until cancellation.
    pub async fn run(self) -> Result<(), DaemonError> {
        let updates = {
            let mut updates = lock(&self.updates);
            updates.take()
        };
        let (sender, mut receiver) = match updates {
            Some(pair) => pair,
            None => {
                self.cancel.cancelled().await;
                self.flush_cache().await?;
                tracing::info!("runtime stopped");
                return Ok(());
            }
        };
        let manager = {
            let mut manager = lock(&self.watch_manager);
            manager.take()
        };

        let rediscover_secs = self.state.config.discovery.rediscover_secs.max(1);
        let mut ticker = tokio::time::interval(Duration::from_secs(rediscover_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        let debounce_duration = Duration::from_millis(self.state.config.watch.debounce_ms.max(1));
        let mut debounce = Box::pin(tokio::time::sleep(IDLE_DEBOUNCE));
        let mut debounce_active = false;

        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => break,
                _ = self.state.refresh.notified() => {
                    if let Some(manager) = &manager {
                        self.rediscover(manager, &sender).await;
                    }
                    if let Err(error) = self.recompute(true).await {
                        tracing::warn!(%error, "manual refresh recompute failed");
                    }
                }
                _ = ticker.tick() => {
                    if let Some(manager) = &manager {
                        self.rediscover(manager, &sender).await;
                    }
                }
                update = receiver.recv() => {
                    if let Some(update) = update {
                        let outcome = self.ingest(update).await;
                        if outcome.changed {
                            debounce.as_mut().reset(tokio::time::Instant::now() + debounce_duration);
                            debounce_active = true;
                        }
                        if outcome.needs_rediscovery
                            && let Some(manager) = &manager
                        {
                            self.rediscover(manager, &sender).await;
                        }
                    }
                }
                _ = &mut debounce, if debounce_active => {
                    debounce_active = false;
                    debounce.as_mut().reset(tokio::time::Instant::now() + IDLE_DEBOUNCE);
                    if let Err(error) = self.recompute(true).await {
                        tracing::warn!(%error, "topology recompute failed");
                    }
                }
            }
        }

        drop(manager);
        self.shutdown_watchers().await;
        self.flush_cache().await?;
        tracing::info!("runtime stopped");
        Ok(())
    }

    async fn shutdown_watchers(&self) {
        self.watch_cancel.cancel();
        let handles = {
            let mut watchers = lock(&self.watchers);
            std::mem::take(&mut *watchers)
        };
        for handle in handles {
            let gvk = handle.gvk.clone();
            handle.stop();
            match tokio::time::timeout(SHUTDOWN_WATCHER_TIMEOUT, handle.task).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::warn!(%gvk, %error, "watcher task failed"),
                Err(_) => tracing::warn!(%gvk, "watcher task did not stop before the timeout"),
            }
        }
    }

    async fn flush_cache(&self) -> Result<(), DaemonError> {
        if !self.state.config.cache.enabled {
            return Ok(());
        }
        let cache = Arc::clone(&self.state.cache);
        match tokio::task::spawn_blocking(move || cache.flush()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(DaemonError::Runtime(format!("cache flush failed: {error}"))),
            Err(error) => Err(DaemonError::Runtime(format!(
                "cache flush task failed: {error}"
            ))),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Compile-time proof that the cache trait is object safe from the daemon side.
#[must_use]
pub fn cache_as_dyn(cache: std::sync::Arc<dyn SnapshotCache>) -> std::sync::Arc<dyn SnapshotCache> {
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use knect_core::config::Config;
    use knect_core::kube::fake::{FakeReader, discovered_resource};
    use knect_core::kube::{WatchEvent, WatchUpdate};
    use knect_proto::events::SseEventKind;
    use knect_proto::{NodeCategory, ResourceId};
    use kube::api::DynamicObject;
    use serde_json::Value;

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/fixtures")
            .join(name)
    }

    fn fixture_objects(name: &str) -> Vec<DynamicObject> {
        let contents = std::fs::read_to_string(fixture_path(name)).expect("fixture readable");
        serde_json::from_str::<Vec<DynamicObject>>(&contents).expect("fixture is valid JSON")
    }

    #[tokio::test]
    async fn fixture_runtime_serves_real_serving_and_eventing_topology() {
        let state = AppState::new(Config::default(), true);
        let cancel = CancellationToken::new();
        let runtime = Runtime::new(
            state.clone(),
            FixtureMode::Enabled(Some(fixture_path(""))),
            cancel.clone(),
        );
        runtime.start().await.expect("start");
        let topology = state.topology();
        for category in [
            NodeCategory::KService,
            NodeCategory::Revision,
            NodeCategory::Broker,
            NodeCategory::Trigger,
        ] {
            assert!(
                topology.nodes.iter().any(|node| node.category == category),
                "missing {category:?} node"
            );
        }
        assert!(
            !topology.edges.is_empty(),
            "fixture topology must include edges"
        );
        assert_eq!(topology.context, "fixture");
        assert!(topology.generation >= 1);
        cancel.cancel();
        runtime.run().await.expect("run");
    }

    #[tokio::test]
    async fn normal_runtime_does_not_fake_a_cluster() {
        let mut config = Config::default();
        config.daemon.context = "not-a-cluster".to_owned();
        config.cache.enabled = false;
        let state = AppState::new(config, false);
        let cancel = CancellationToken::new();
        let runtime = Runtime::new(state.clone(), FixtureMode::Disabled, cancel.clone())
            .with_kubeconfig(Some(PathBuf::from("/nonexistent/knect-test-kubeconfig")));
        runtime.start().await.expect("start");
        assert_eq!(state.topology().context, "not-a-cluster");
        assert!(state.topology().nodes.is_empty());
        cancel.cancel();
        runtime.run().await.expect("run");
    }

    #[tokio::test]
    async fn fake_watch_updates_produce_a_topology_and_a_graph_patch() {
        let mut config = Config::default();
        config.cache.enabled = false;
        config.discovery.groups.clear();
        let state = AppState::new(config, false);
        let cancel = CancellationToken::new();

        let objects = fixture_objects("serving/serving.json");
        let mut script = vec![Ok(WatchEvent::Init)];
        for object in objects {
            script.push(Ok(WatchEvent::InitApply(object)));
        }
        script.push(Ok(WatchEvent::InitDone));
        let reader = FakeReader::new()
            .with_discovery(DiscoverySnapshot {
                resources: vec![discovered_resource(
                    "serving.knative.dev",
                    "v1",
                    "Service",
                    "services",
                )],
                warnings: Vec::new(),
            })
            .with_watch(script);

        let runtime = Runtime::new(state.clone(), FixtureMode::Disabled, cancel.clone())
            .with_cluster_reader(Arc::new(reader));
        let mut events = state.events.subscribe();
        runtime.start().await.expect("start");
        let run = tokio::spawn(runtime.run());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while state.topology().nodes.is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let topology = state.topology();
        assert!(
            !topology.nodes.is_empty(),
            "watch updates must build a graph"
        );
        assert!(
            topology
                .nodes
                .iter()
                .any(|node| node.category == NodeCategory::KService)
        );

        let event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if matches!(event.kind, SseEventKind::GraphPatch { .. }) {
                            break event;
                        }
                    }
                    Err(_) => continue,
                }
            }
        })
        .await
        .expect("a graph patch must be broadcast");
        match event.kind {
            SseEventKind::GraphPatch { data } => {
                assert_eq!(data.seq, event.seq);
                assert!(
                    !data.added_nodes.is_empty(),
                    "first patch adds the discovered nodes"
                );
                assert!(
                    data.added_nodes
                        .iter()
                        .any(|node| node.id.kind == "Service")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }

        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("runtime stops")
            .expect("join")
            .expect("run");
    }

    #[test]
    fn resource_ids_in_fixtures_round_trip() {
        let id = ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            "knect-e2e",
            "order-processor",
        );
        assert_eq!(id.kind, "Service");
        let value: Value = serde_json::to_value(&id).expect("serialize id");
        assert_eq!(value["name"], "order-processor");
    }

    #[tokio::test]
    async fn watch_update_ingest_sanitizes_secret_material() {
        let state = AppState::new(Config::default(), false);
        let cancel = CancellationToken::new();
        let runtime = Runtime::new(state.clone(), FixtureMode::Disabled, cancel.clone());
        let mut object = DynamicObject::new(
            "credentials",
            &kube::api::ApiResource {
                group: String::new(),
                version: "v1".to_owned(),
                api_version: "v1".to_owned(),
                kind: "Secret".to_owned(),
                plural: "secrets".to_owned(),
            },
        );
        object.metadata.namespace = Some("default".to_owned());
        object.data = serde_json::json!({"data": {"password": "aHVudGVyMi1rbmVjdA=="}});
        let id = resource_id(&object);
        let outcome = runtime
            .ingest(WatchUpdate::Apply {
                gvk: Gvk::new("", "v1", "Secret"),
                id: id.clone(),
                object,
            })
            .await;
        assert!(outcome.changed);
        let stored = state.store.read().await.get(&id).expect("stored");
        let serialized = serde_json::to_string(&*stored).expect("serialize");
        assert!(!serialized.contains("aHVudGVyMi1rbmVjdA=="));
        assert!(state.secret_material.get(&id).is_some());

        let outcome = runtime
            .ingest(WatchUpdate::Delete {
                gvk: Gvk::new("", "v1", "Secret"),
                id: id.clone(),
                object: DynamicObject::new(
                    "credentials",
                    &kube::api::ApiResource {
                        group: String::new(),
                        version: "v1".to_owned(),
                        api_version: "v1".to_owned(),
                        kind: "Secret".to_owned(),
                        plural: "secrets".to_owned(),
                    },
                ),
            })
            .await;
        assert!(outcome.changed);
        assert!(state.store.read().await.get(&id).is_none());
        assert!(state.secret_material.get(&id).is_none());
    }
}
