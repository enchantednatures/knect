//! Supervised watch streams.
//!
//! One task owns each discovered resource. The task consumes the reader watch
//! stream, maps runtime events to [`WatchUpdate`] values, and retries transient
//! failures with deterministic exponential backoff. A `404` from the API server
//! means the CRD disappeared; the task stops and reports
//! [`WatchOutcome::ResourceGone`] so the daemon can rediscover.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{ClusterReader, DiscoveredResource, WatchEvent};
use crate::config::WatchConfig;
use crate::error::CoreError;
use crate::store::resource_id;

/// Default first retry delay.
pub const DEFAULT_BACKOFF_BASE: Duration = Duration::from_millis(200);
/// Default retry delay ceiling.
pub const DEFAULT_BACKOFF_CAP: Duration = Duration::from_secs(30);
/// Default consecutive failures before a watcher gives up.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 8;

/// Deterministic exponential backoff policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackoffPolicy {
    base: Duration,
    cap: Duration,
    max_attempts: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base: DEFAULT_BACKOFF_BASE,
            cap: DEFAULT_BACKOFF_CAP,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

impl BackoffPolicy {
    /// Creates a policy. `max_attempts` is the number of retries allowed after
    /// the first failure; zero disables retries.
    #[must_use]
    pub fn new(base: Duration, cap: Duration, max_attempts: u32) -> Self {
        Self {
            base,
            cap,
            max_attempts,
        }
    }

    /// Returns the delay before retry number `attempt` (1-based), or `None`
    /// when the attempt budget is exhausted.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }
        let factor = 2_u32.saturating_pow(attempt.saturating_sub(1));
        Some(self.base.saturating_mul(factor).min(self.cap))
    }

    /// Returns the full retry schedule for tests and diagnostics.
    #[must_use]
    pub fn schedule(&self) -> Vec<Duration> {
        (1..=self.max_attempts)
            .filter_map(|attempt| self.delay(attempt))
            .collect()
    }

    /// Maximum number of retries.
    #[must_use]
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
}

/// A typed update emitted by a watcher task.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchUpdate {
    /// The watcher restarted its initial list.
    Init {
        /// Watched resource.
        gvk: Gvk,
    },
    /// An object arrived during the initial list.
    InitApply {
        /// Watched resource.
        gvk: Gvk,
        /// Object identity.
        id: ResourceId,
        /// The object.
        object: DynamicObject,
    },
    /// An object was added or modified.
    Apply {
        /// Watched resource.
        gvk: Gvk,
        /// Object identity.
        id: ResourceId,
        /// The object.
        object: DynamicObject,
    },
    /// An object was deleted.
    Delete {
        /// Watched resource.
        gvk: Gvk,
        /// Object identity.
        id: ResourceId,
        /// The object.
        object: DynamicObject,
    },
    /// The initial list completed.
    InitDone {
        /// Watched resource.
        gvk: Gvk,
    },
    /// A transient failure is being retried.
    Retry {
        /// Watched resource.
        gvk: Gvk,
        /// Consecutive failure count starting at one.
        attempt: u32,
        /// Delay applied before the next attempt.
        delay_ms: u64,
    },
    /// The API server returned `404`; the resource should be rediscovered.
    ResourceGone {
        /// Watched resource.
        gvk: Gvk,
        /// Server message.
        message: String,
    },
    /// The retry budget is exhausted.
    RetriesExhausted {
        /// Watched resource.
        gvk: Gvk,
        /// Consecutive failures observed.
        attempts: u32,
    },
}

/// Terminal state of one watcher task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchOutcome {
    /// The task stopped because its cancellation token fired.
    Cancelled {
        /// Watched resource.
        gvk: Gvk,
    },
    /// The API server returned `404`; the resource should be rediscovered.
    ResourceGone {
        /// Watched resource.
        gvk: Gvk,
        /// Server message.
        message: String,
    },
    /// The retry budget is exhausted.
    RetriesExhausted {
        /// Watched resource.
        gvk: Gvk,
        /// Consecutive failures observed.
        attempts: u32,
    },
    /// The update receiver was dropped.
    ConsumerClosed {
        /// Watched resource.
        gvk: Gvk,
    },
    /// The task panicked or was aborted.
    TaskFailed {
        /// Watched resource.
        gvk: Gvk,
        /// Join error text.
        message: String,
    },
}

/// A running watcher task.
pub struct WatchHandle {
    /// Watched resource.
    pub gvk: Gvk,
    /// Per-resource cancellation token.
    pub cancel: CancellationToken,
    /// Task that resolves once the watcher stops.
    pub task: JoinHandle<WatchOutcome>,
}

impl WatchHandle {
    /// Requests a clean stop without aborting the task.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// Returns true when a stop was requested.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.task.is_finished() || self.cancel.is_cancelled()
    }
}

/// Spawns and supervises one watcher per discovered resource.
#[derive(Clone)]
pub struct WatchManager {
    reader: Arc<dyn ClusterReader>,
    config: WatchConfig,
    backoff: BackoffPolicy,
}

impl WatchManager {
    /// Creates a manager with the default backoff policy.
    #[must_use]
    pub fn new(reader: Arc<dyn ClusterReader>, config: WatchConfig) -> Self {
        Self::with_backoff(reader, config, BackoffPolicy::default())
    }

    /// Creates a manager with an explicit backoff policy.
    #[must_use]
    pub fn with_backoff(
        reader: Arc<dyn ClusterReader>,
        config: WatchConfig,
        backoff: BackoffPolicy,
    ) -> Self {
        Self {
            reader,
            config,
            backoff,
        }
    }

    /// Returns the watch configuration.
    #[must_use]
    pub fn config(&self) -> &WatchConfig {
        &self.config
    }

    /// Returns the retry policy.
    #[must_use]
    pub fn backoff(&self) -> &BackoffPolicy {
        &self.backoff
    }

    /// Starts one watcher per resource and returns its handle.
    ///
    /// Watchers cover all namespaces; namespace scoping belongs to the
    /// discovery/watch supervisor in Task 5.
    #[must_use]
    pub fn spawn(
        &self,
        resources: &[DiscoveredResource],
        cancel: CancellationToken,
    ) -> Vec<WatchHandle> {
        self.spawn_internal(resources, cancel, None)
    }

    /// Starts watchers and forwards every update to `updates`.
    #[must_use]
    pub fn spawn_with_updates(
        &self,
        resources: &[DiscoveredResource],
        cancel: CancellationToken,
        updates: mpsc::Sender<WatchUpdate>,
    ) -> Vec<WatchHandle> {
        self.spawn_internal(resources, cancel, Some(updates))
    }

    /// Runs every watcher to completion and returns their terminal outcomes.
    pub async fn run(
        &self,
        resources: &[DiscoveredResource],
        cancel: CancellationToken,
        updates: mpsc::Sender<WatchUpdate>,
    ) -> Vec<WatchOutcome> {
        let handles = self.spawn_with_updates(resources, cancel, updates);
        let mut outcomes = Vec::with_capacity(handles.len());
        for handle in handles {
            let gvk = handle.gvk.clone();
            match handle.task.await {
                Ok(outcome) => outcomes.push(outcome),
                Err(error) => outcomes.push(WatchOutcome::TaskFailed {
                    gvk,
                    message: error.to_string(),
                }),
            }
        }
        outcomes
    }

    fn spawn_internal(
        &self,
        resources: &[DiscoveredResource],
        cancel: CancellationToken,
        updates: Option<mpsc::Sender<WatchUpdate>>,
    ) -> Vec<WatchHandle> {
        resources
            .iter()
            .map(|resource| {
                let gvk = resource.gvk.clone();
                let token = cancel.child_token();
                let task = tokio::spawn(supervise(
                    Arc::clone(&self.reader),
                    resource.clone(),
                    token.clone(),
                    self.backoff.clone(),
                    updates.clone(),
                ));
                WatchHandle {
                    gvk,
                    cancel: token,
                    task,
                }
            })
            .collect()
    }
}

async fn supervise(
    reader: Arc<dyn ClusterReader>,
    resource: DiscoveredResource,
    cancel: CancellationToken,
    policy: BackoffPolicy,
    updates: Option<mpsc::Sender<WatchUpdate>>,
) -> WatchOutcome {
    let gvk = resource.gvk.clone();
    let ar = resource.api_resource();
    let mut failures = 0_u32;

    loop {
        if cancel.is_cancelled() {
            return WatchOutcome::Cancelled { gvk };
        }
        let stream_cancel = cancel.child_token();
        let mut stream = reader.watch(&ar, None, stream_cancel);
        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => return WatchOutcome::Cancelled { gvk },
                next = stream.next() => next,
            };
            match next {
                Some(Ok(event)) => {
                    failures = 0;
                    let Some(update) = event_to_update(&gvk, event) else {
                        continue;
                    };
                    if !emit(&updates, update).await {
                        return WatchOutcome::ConsumerClosed { gvk };
                    }
                }
                Some(Err(error)) => {
                    if is_resource_gone(&error) {
                        let message = error.to_string();
                        if !emit(
                            &updates,
                            WatchUpdate::ResourceGone {
                                gvk: gvk.clone(),
                                message: message.clone(),
                            },
                        )
                        .await
                        {
                            return WatchOutcome::ConsumerClosed { gvk };
                        }
                        return WatchOutcome::ResourceGone { gvk, message };
                    }
                    match retry_after(&policy, &mut failures, &gvk, &updates, &cancel).await {
                        RetryStep::Retry => {}
                        RetryStep::Stop(outcome) => return outcome,
                    }
                }
                None => match retry_after(&policy, &mut failures, &gvk, &updates, &cancel).await {
                    RetryStep::Retry => break,
                    RetryStep::Stop(outcome) => return outcome,
                },
            }
        }
    }
}

enum RetryStep {
    Retry,
    Stop(WatchOutcome),
}

async fn retry_after(
    policy: &BackoffPolicy,
    failures: &mut u32,
    gvk: &Gvk,
    updates: &Option<mpsc::Sender<WatchUpdate>>,
    cancel: &CancellationToken,
) -> RetryStep {
    *failures = failures.saturating_add(1);
    let Some(delay) = policy.delay(*failures) else {
        if !emit(
            updates,
            WatchUpdate::RetriesExhausted {
                gvk: gvk.clone(),
                attempts: *failures,
            },
        )
        .await
        {
            return RetryStep::Stop(WatchOutcome::ConsumerClosed { gvk: gvk.clone() });
        }
        return RetryStep::Stop(WatchOutcome::RetriesExhausted {
            gvk: gvk.clone(),
            attempts: *failures,
        });
    };
    let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
    if !emit(
        updates,
        WatchUpdate::Retry {
            gvk: gvk.clone(),
            attempt: *failures,
            delay_ms,
        },
    )
    .await
    {
        return RetryStep::Stop(WatchOutcome::ConsumerClosed { gvk: gvk.clone() });
    }
    let paused = tokio::select! {
        biased;
        () = cancel.cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    };
    if paused {
        RetryStep::Retry
    } else {
        RetryStep::Stop(WatchOutcome::Cancelled { gvk: gvk.clone() })
    }
}

async fn emit(updates: &Option<mpsc::Sender<WatchUpdate>>, update: WatchUpdate) -> bool {
    match updates {
        Some(sender) => sender.send(update).await.is_ok(),
        None => true,
    }
}

fn event_to_update(gvk: &Gvk, event: WatchEvent) -> Option<WatchUpdate> {
    match event {
        WatchEvent::Init => Some(WatchUpdate::Init { gvk: gvk.clone() }),
        WatchEvent::InitApply(object) => {
            let object = stamp_gvk(object, gvk);
            Some(WatchUpdate::InitApply {
                gvk: gvk.clone(),
                id: resource_id(&object),
                object,
            })
        }
        WatchEvent::Apply(object) => {
            let object = stamp_gvk(object, gvk);
            Some(WatchUpdate::Apply {
                gvk: gvk.clone(),
                id: resource_id(&object),
                object,
            })
        }
        WatchEvent::Delete(object) => {
            let object = stamp_gvk(object, gvk);
            Some(WatchUpdate::Delete {
                gvk: gvk.clone(),
                id: resource_id(&object),
                object,
            })
        }
        WatchEvent::InitDone => Some(WatchUpdate::InitDone { gvk: gvk.clone() }),
    }
}

/// Kubernetes watch events can omit `TypeMeta`. Stamp the discovered GVK so
/// every object enters the store under the same identity its watcher reports.
fn stamp_gvk(mut object: DynamicObject, gvk: &Gvk) -> DynamicObject {
    object.types = Some(kube::api::TypeMeta {
        api_version: gvk.api_version(),
        kind: gvk.kind.clone(),
    });
    object
}

fn is_resource_gone(error: &CoreError) -> bool {
    match error {
        CoreError::Kube(kube::Error::Api(status)) => {
            status.code == 404 || status.reason.eq_ignore_ascii_case("NotFound")
        }
        CoreError::Watch(message) | CoreError::Discovery(message) => {
            message_indicates_gone(message)
        }
        _ => false,
    }
}

fn message_indicates_gone(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("404")
        || lower.contains("not found")
        || lower.contains("notfound")
        || lower.contains("could not find the requested resource")
        || lower.contains("no matches for kind")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WatchConfig;
    use crate::kube::fake::{FakeReader, not_found_error};
    use kube::ResourceExt;
    use kube::api::ApiResource;

    fn discovered(group: &str, version: &str, kind: &str) -> DiscoveredResource {
        DiscoveredResource {
            gvk: Gvk::new(group, version, kind),
            plural: format!("{}s", kind.to_ascii_lowercase()),
            api_version: Gvk::new(group, version, kind).api_version(),
            namespaced: true,
            verbs: vec!["list".to_owned(), "watch".to_owned()],
        }
    }

    fn test_object(kind: &str, name: &str) -> DynamicObject {
        DynamicObject::new(
            name,
            &ApiResource {
                group: String::new(),
                version: "v1".to_owned(),
                api_version: "v1".to_owned(),
                kind: kind.to_owned(),
                plural: format!("{}s", kind.to_ascii_lowercase()),
            },
        )
    }

    #[test]
    fn backoff_schedule_is_deterministic_and_capped() {
        let policy = BackoffPolicy::new(Duration::from_millis(200), Duration::from_secs(30), 10);
        let schedule = policy.schedule();
        assert_eq!(schedule.len(), 10);
        assert_eq!(schedule[0], Duration::from_millis(200));
        assert_eq!(schedule[1], Duration::from_millis(400));
        assert_eq!(schedule[2], Duration::from_millis(800));
        assert_eq!(schedule[9], Duration::from_secs(30));
        assert!(schedule.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(schedule.iter().all(|delay| *delay <= policy_cap()));
        assert!(policy.delay(0).is_none());
        assert!(policy.delay(11).is_none());
    }

    fn policy_cap() -> Duration {
        Duration::from_secs(30)
    }

    #[test]
    fn not_found_is_distinguished_from_transient_errors() {
        assert!(is_resource_gone(&not_found_error("services.example.com")));
        assert!(is_resource_gone(&CoreError::Watch(
            "watch failed: 404 not found".to_owned()
        )));
        assert!(!is_resource_gone(&CoreError::Watch(
            "connection reset by peer".to_owned()
        )));
    }

    #[test]
    fn runtime_events_map_to_typed_updates() {
        let gvk = Gvk::new("v1", "v1", "ConfigMap");
        assert!(matches!(
            event_to_update(&gvk, WatchEvent::Init),
            Some(WatchUpdate::Init { .. })
        ));
        let apply = event_to_update(&gvk, WatchEvent::Apply(test_object("ConfigMap", "one")));
        match apply {
            Some(WatchUpdate::Apply { id, object, .. }) => {
                assert_eq!(id.name, "one");
                assert_eq!(object.name_any(), "one");
            }
            other => panic!("unexpected update: {other:?}"),
        }

        // kube-rs watch events may omit TypeMeta. The watcher GVK must still
        // become the object identity, or core resources never reach the store.
        let core_gvk = Gvk::new("", "v1", "Pod");
        let mut typeless = test_object("Pod", "web");
        typeless.types = None;
        typeless.metadata.namespace = Some("default".to_owned());
        match event_to_update(&core_gvk, WatchEvent::InitApply(typeless)) {
            Some(WatchUpdate::InitApply { id, object, .. }) => {
                assert_eq!(id.api_version, "v1");
                assert_eq!(id.kind, "Pod");
                assert_eq!(id.namespace.as_deref(), Some("default"));
                assert_eq!(id.name, "web");
                assert_eq!(
                    object.types.as_ref().map(|types| types.kind.as_str()),
                    Some("Pod")
                );
            }
            other => panic!("unexpected update: {other:?}"),
        }

        assert!(matches!(
            event_to_update(&gvk, WatchEvent::Delete(test_object("ConfigMap", "one"))),
            Some(WatchUpdate::Delete { .. })
        ));
        assert!(matches!(
            event_to_update(&gvk, WatchEvent::InitDone),
            Some(WatchUpdate::InitDone { .. })
        ));
    }

    fn manager(reader: FakeReader, max_attempts: u32) -> WatchManager {
        WatchManager::with_backoff(
            Arc::new(reader),
            WatchConfig::default(),
            BackoffPolicy::new(
                Duration::from_millis(1),
                Duration::from_millis(2),
                max_attempts,
            ),
        )
    }

    #[tokio::test]
    async fn cancellation_stops_the_task_cleanly() {
        let reader = FakeReader::new();
        let manager = manager(reader, 4);
        let cancel = CancellationToken::new();
        let mut handles = manager.spawn(&[discovered("v1", "v1", "ConfigMap")], cancel.clone());
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(2), &mut handles[0].task)
            .await
            .expect("watcher task must stop")
            .expect("watcher task must not panic");
        assert!(matches!(outcome, WatchOutcome::Cancelled { .. }));
    }

    #[tokio::test]
    async fn not_found_maps_to_resource_gone() {
        let reader = FakeReader::new().with_watch(vec![Err(not_found_error(
            "customresourcedefinitions.apiextensions.k8s.io",
        ))]);
        let manager = manager(reader, 4);
        let mut handles = manager.spawn(
            &[discovered("example.com", "v1", "Widget")],
            CancellationToken::new(),
        );
        let outcome = tokio::time::timeout(Duration::from_secs(2), &mut handles[0].task)
            .await
            .expect("watcher task must stop")
            .expect("watcher task must not panic");
        assert!(matches!(outcome, WatchOutcome::ResourceGone { .. }));
    }

    #[tokio::test]
    async fn transient_errors_retry_until_an_event_arrives() {
        let object = test_object("ConfigMap", "recovered");
        let reader = FakeReader::new().with_watch(vec![
            Err(CoreError::Watch("connection reset".to_owned())),
            Err(CoreError::Watch("connection reset".to_owned())),
            Ok(WatchEvent::Apply(object)),
        ]);
        let manager = manager(reader, 4);
        let cancel = CancellationToken::new();
        let (sender, mut receiver) = mpsc::channel(16);
        let mut handles = manager.spawn_with_updates(
            &[discovered("v1", "v1", "ConfigMap")],
            cancel.clone(),
            sender,
        );
        let update = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match receiver.recv().await {
                    Some(WatchUpdate::Apply { .. }) => return Some(()),
                    Some(_) => {}
                    None => return None,
                }
            }
        })
        .await
        .expect("apply update must arrive");
        assert!(update.is_some());
        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut handles[0].task).await;
    }

    #[tokio::test]
    async fn exhausted_retries_stop_with_a_typed_outcome() {
        let reader = FakeReader::new().with_watch(vec![
            Err(CoreError::Watch("connection reset".to_owned())),
            Err(CoreError::Watch("connection reset".to_owned())),
        ]);
        let manager = manager(reader, 1);
        let mut handles = manager.spawn(
            &[discovered("v1", "v1", "ConfigMap")],
            CancellationToken::new(),
        );
        let outcome = tokio::time::timeout(Duration::from_secs(2), &mut handles[0].task)
            .await
            .expect("watcher task must stop")
            .expect("watcher task must not panic");
        assert!(matches!(
            outcome,
            WatchOutcome::RetriesExhausted { attempts: 2, .. }
        ));
    }
}
