//! Deterministic in-memory [`ClusterReader`] for tests.
//!
//! Every scripted call is recorded so tests can assert both the response and
//! the exact request that produced it. Watch scripts are consumed by the first
//! call; a supervisor that reconnects sees an empty stream and exercises its
//! backoff path instead of replaying stale events.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::StreamExt;
use futures::stream::BoxStream;
use knect_proto::{Gvk, ResourceId};
use kube::api::{ApiResource, DynamicObject};
use tokio_util::sync::CancellationToken;

use super::reader::dynamic_resource;
use super::{
    ClusterReader, CoreFuture, DiscoveredResource, DiscoverySnapshot, DryRunResult, LogOptions,
    WatchEvent,
};
use crate::error::CoreError;
use crate::store::resource_id;

/// One recorded `list` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCall {
    /// Requested resource.
    pub resource: ApiResource,
    /// Requested namespace, or `None` for all namespaces.
    pub namespace: Option<String>,
}

/// One recorded `get` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetCall {
    /// Requested group, version, and kind.
    pub gvk: Gvk,
    /// Requested namespace.
    pub namespace: Option<String>,
    /// Requested object name.
    pub name: String,
}

/// One recorded `patch_merge` call.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchCall {
    /// Patched group, version, and kind.
    pub gvk: Gvk,
    /// Patched namespace.
    pub namespace: String,
    /// Patched object name.
    pub name: String,
    /// Merge patch payload.
    pub patch: serde_json::Value,
    /// Whether the request was a dry run.
    pub dry_run: bool,
}

/// One recorded `watch` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchCall {
    /// Watched resource.
    pub resource: ApiResource,
    /// Watched namespace, or `None` for all namespaces.
    pub namespace: Option<String>,
}

/// One recorded `delete` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteCall {
    /// Deleted group, version, and kind.
    pub gvk: Gvk,
    /// Deleted namespace.
    pub namespace: Option<String>,
    /// Deleted object name.
    pub name: String,
}

/// One recorded `log_stream` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogCall {
    /// Pod namespace.
    pub namespace: String,
    /// Pod name.
    pub pod: String,
    /// Requested log options.
    pub options: LogOptions,
}

/// Every request recorded by a [`FakeReader`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordedCalls {
    /// Number of discovery calls.
    pub discovery: usize,
    /// List calls in order.
    pub list: Vec<ListCall>,
    /// Get calls in order.
    pub get: Vec<GetCall>,
    /// Patch calls in order.
    pub patch: Vec<PatchCall>,
    /// Dry-run create payloads in order.
    pub dry_run_create: Vec<DynamicObject>,
    /// Watch calls in order.
    pub watch: Vec<WatchCall>,
    /// Created object payloads in order.
    pub create: Vec<DynamicObject>,
    /// Delete calls in order.
    pub delete: Vec<DeleteCall>,
    /// Log stream calls in order.
    pub log_stream: Vec<LogCall>,
}

#[derive(Default)]
struct FakeState {
    discovery: DiscoverySnapshot,
    discovery_error: Option<CoreError>,
    objects: HashMap<ResourceId, DynamicObject>,
    lists: HashMap<(ApiResource, Option<String>), Vec<DynamicObject>>,
    watch_script: VecDeque<Result<WatchEvent, CoreError>>,
    patch_responses: VecDeque<Result<DynamicObject, CoreError>>,
    dry_run_responses: VecDeque<Result<DryRunResult, CoreError>>,
    create_responses: VecDeque<Result<DynamicObject, CoreError>>,
    delete_responses: VecDeque<Result<(), CoreError>>,
    log_script: VecDeque<Result<String, CoreError>>,
    calls: RecordedCalls,
}

/// Scriptable `ClusterReader`.
#[derive(Clone, Default)]
pub struct FakeReader {
    state: Arc<Mutex<FakeState>>,
}

impl FakeReader {
    /// Creates a reader with empty discovery, no objects, and empty scripts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the discovery snapshot returned by [`ClusterReader::discover`].
    #[must_use]
    pub fn with_discovery(self, snapshot: DiscoverySnapshot) -> Self {
        {
            let mut state = self.lock();
            state.discovery = snapshot;
            state.discovery_error = None;
        }
        self
    }

    /// Makes discovery fail with `error`.
    #[must_use]
    pub fn fail_discovery(self, error: CoreError) -> Self {
        self.lock().discovery_error = Some(error);
        self
    }

    /// Adds one object to the get map and to its derived list bucket.
    #[must_use]
    pub fn with_object(self, object: DynamicObject) -> Self {
        let id = resource_id(&object);
        let resource = dynamic_resource(&id.gvk());
        let namespace = id.namespace.clone();
        {
            let mut state = self.lock();
            state
                .lists
                .entry((resource, namespace))
                .or_default()
                .push(object.clone());
            state.objects.insert(id, object);
        }
        self
    }

    /// Adds one object to an explicit list bucket.
    #[must_use]
    pub fn with_object_in(
        self,
        resource: ApiResource,
        namespace: Option<&str>,
        mut object: DynamicObject,
    ) -> Self {
        if object.metadata.namespace.is_none()
            && let Some(namespace) = namespace
        {
            object.metadata.namespace = Some(namespace.to_owned());
        }
        let id = resource_id(&object);
        {
            let mut state = self.lock();
            state
                .lists
                .entry((resource, namespace.map(ToOwned::to_owned)))
                .or_default()
                .push(object.clone());
            state.objects.insert(id, object);
        }
        self
    }

    /// Replaces the watch script.
    #[must_use]
    pub fn with_watch(self, script: Vec<Result<WatchEvent, CoreError>>) -> Self {
        self.lock().watch_script = script.into();
        self
    }

    /// Appends one scripted patch response.
    #[must_use]
    pub fn with_patch_response(self, response: Result<DynamicObject, CoreError>) -> Self {
        self.lock().patch_responses.push_back(response);
        self
    }

    /// Appends one scripted dry-run create response.
    #[must_use]
    pub fn with_dry_run_response(self, response: Result<DryRunResult, CoreError>) -> Self {
        self.lock().dry_run_responses.push_back(response);
        self
    }

    /// Appends one scripted create response. An unscripted create stores and
    /// returns the supplied object.
    #[must_use]
    pub fn with_create_response(self, response: Result<DynamicObject, CoreError>) -> Self {
        self.lock().create_responses.push_back(response);
        self
    }

    /// Appends one scripted delete response. An unscripted delete removes the
    /// object from every index and succeeds.
    #[must_use]
    pub fn with_delete_response(self, response: Result<(), CoreError>) -> Self {
        self.lock().delete_responses.push_back(response);
        self
    }

    /// Replaces the pod-log script.
    #[must_use]
    pub fn with_log_stream(self, script: Vec<Result<String, CoreError>>) -> Self {
        self.lock().log_script = script.into();
        self
    }

    /// Returns every recorded call.
    #[must_use]
    pub fn calls(&self) -> RecordedCalls {
        self.lock().calls.clone()
    }

    /// Returns the number of discovery calls.
    #[must_use]
    pub fn discovery_calls(&self) -> usize {
        self.lock().calls.discovery
    }

    /// Returns recorded list calls.
    #[must_use]
    pub fn list_calls(&self) -> Vec<ListCall> {
        self.lock().calls.list.clone()
    }

    /// Returns recorded get calls.
    #[must_use]
    pub fn get_calls(&self) -> Vec<GetCall> {
        self.lock().calls.get.clone()
    }

    /// Returns recorded patch calls.
    #[must_use]
    pub fn patch_calls(&self) -> Vec<PatchCall> {
        self.lock().calls.patch.clone()
    }

    /// Returns recorded dry-run create payloads.
    #[must_use]
    pub fn dry_run_calls(&self) -> Vec<DynamicObject> {
        self.lock().calls.dry_run_create.clone()
    }

    /// Returns recorded watch calls.
    #[must_use]
    pub fn watch_calls(&self) -> Vec<WatchCall> {
        self.lock().calls.watch.clone()
    }

    /// Returns recorded create payloads.
    #[must_use]
    pub fn create_calls(&self) -> Vec<DynamicObject> {
        self.lock().calls.create.clone()
    }

    /// Returns recorded delete calls.
    #[must_use]
    pub fn delete_calls(&self) -> Vec<DeleteCall> {
        self.lock().calls.delete.clone()
    }

    /// Returns recorded log stream calls.
    #[must_use]
    pub fn log_calls(&self) -> Vec<LogCall> {
        self.lock().calls.log_stream.clone()
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ClusterReader for FakeReader {
    fn discover(&self) -> CoreFuture<'_, Result<DiscoverySnapshot, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.discovery = state.calls.discovery.saturating_add(1);
            if let Some(error) = state.discovery_error.take() {
                return Err(error);
            }
            Ok(state.discovery.clone())
        })
    }

    fn list<'a>(
        &'a self,
        ar: &'a ApiResource,
        ns: Option<&'a str>,
    ) -> CoreFuture<'a, Result<Vec<DynamicObject>, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.list.push(ListCall {
                resource: ar.clone(),
                namespace: ns.map(ToOwned::to_owned),
            });
            let key = (ar.clone(), ns.map(ToOwned::to_owned));
            if let Some(objects) = state.lists.get(&key) {
                let mut objects = objects.clone();
                sort_objects(&mut objects);
                return Ok(objects);
            }
            let mut objects: Vec<DynamicObject> = state
                .objects
                .values()
                .filter(|object| object_matches(object, ar, ns))
                .cloned()
                .collect();
            sort_objects(&mut objects);
            Ok(objects)
        })
    }

    fn watch(
        &self,
        ar: &ApiResource,
        ns: Option<&str>,
        _cancel: CancellationToken,
    ) -> BoxStream<'static, Result<WatchEvent, CoreError>> {
        let mut state = self.lock();
        state.calls.watch.push(WatchCall {
            resource: ar.clone(),
            namespace: ns.map(ToOwned::to_owned),
        });
        let script: Vec<Result<WatchEvent, CoreError>> = state.watch_script.drain(..).collect();
        futures::stream::iter(script).boxed()
    }

    fn get<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: Option<&'a str>,
        name: &'a str,
    ) -> CoreFuture<'a, Result<DynamicObject, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.get.push(GetCall {
                gvk: gvk.clone(),
                namespace: ns.map(ToOwned::to_owned),
                name: name.to_owned(),
            });
            let id = ResourceId::new(
                gvk.api_version(),
                gvk.kind.clone(),
                ns.map(ToOwned::to_owned),
                name,
            );
            state
                .objects
                .get(&id)
                .cloned()
                .ok_or_else(|| not_found_error(id.to_string()))
        })
    }

    fn patch_merge<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: &'a str,
        name: &'a str,
        patch: serde_json::Value,
        dry_run: bool,
    ) -> CoreFuture<'a, Result<DynamicObject, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.patch.push(PatchCall {
                gvk: gvk.clone(),
                namespace: ns.to_owned(),
                name: name.to_owned(),
                patch,
                dry_run,
            });
            state.patch_responses.pop_front().ok_or_else(|| {
                CoreError::Internal("FakeReader has no scripted patch response".to_owned())
            })?
        })
    }

    fn dry_run_create(
        &self,
        obj: DynamicObject,
    ) -> CoreFuture<'_, Result<DryRunResult, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.dry_run_create.push(obj);
            state.dry_run_responses.pop_front().ok_or_else(|| {
                CoreError::Internal("FakeReader has no scripted dry-run response".to_owned())
            })?
        })
    }

    fn create(&self, obj: DynamicObject) -> CoreFuture<'_, Result<DynamicObject, CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.create.push(obj.clone());
            if let Some(response) = state.create_responses.pop_front() {
                return response;
            }
            let id = resource_id(&obj);
            let resource = dynamic_resource(&id.gvk());
            let namespace = id.namespace.clone();
            state
                .lists
                .entry((resource, namespace))
                .or_default()
                .push(obj.clone());
            state.objects.insert(id, obj.clone());
            Ok(obj)
        })
    }

    fn delete<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: Option<&'a str>,
        name: &'a str,
    ) -> CoreFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.calls.delete.push(DeleteCall {
                gvk: gvk.clone(),
                namespace: ns.map(ToOwned::to_owned),
                name: name.to_owned(),
            });
            if let Some(response) = state.delete_responses.pop_front() {
                return response;
            }
            let id = ResourceId::new(
                gvk.api_version(),
                gvk.kind.clone(),
                ns.map(ToOwned::to_owned),
                name,
            );
            state.objects.remove(&id);
            for objects in state.lists.values_mut() {
                objects.retain(|object| resource_id(object) != id);
            }
            Ok(())
        })
    }

    fn log_stream(
        &self,
        ns: &str,
        pod: &str,
        opts: LogOptions,
    ) -> BoxStream<'static, Result<String, CoreError>> {
        let mut state = self.lock();
        state.calls.log_stream.push(LogCall {
            namespace: ns.to_owned(),
            pod: pod.to_owned(),
            options: opts,
        });
        let script: Vec<Result<String, CoreError>> = state.log_script.drain(..).collect();
        futures::stream::iter(script).boxed()
    }
}

/// Builds a typed `404 NotFound` error for watch and get scripts.
#[must_use]
pub fn not_found_error(resource: impl Into<String>) -> CoreError {
    CoreError::Kube(kube::Error::Api(
        kube::core::Status {
            code: 404,
            message: format!("{} not found", resource.into()),
            reason: "NotFound".to_owned(),
            ..kube::core::Status::default()
        }
        .boxed(),
    ))
}

/// Builds a discovered resource with `list` and `watch` verbs.
#[must_use]
pub fn discovered_resource(
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
) -> DiscoveredResource {
    DiscoveredResource {
        gvk: Gvk::new(group, version, kind),
        plural: plural.to_owned(),
        api_version: Gvk::new(group, version, kind).api_version(),
        namespaced: true,
        verbs: vec!["list".to_owned(), "watch".to_owned()],
    }
}

fn object_matches(object: &DynamicObject, ar: &ApiResource, ns: Option<&str>) -> bool {
    let id = resource_id(object);
    let gvk = id.gvk();
    gvk.group == ar.group
        && gvk.version == ar.version
        && gvk.kind == ar.kind
        && ns.is_none_or(|namespace| id.namespace.as_deref() == Some(namespace))
}

fn sort_objects(objects: &mut [DynamicObject]) {
    objects.sort_by_key(resource_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::TypeMeta;

    fn object(kind: &str, namespace: &str, name: &str) -> DynamicObject {
        DynamicObject {
            types: Some(TypeMeta {
                api_version: "v1".to_owned(),
                kind: kind.to_owned(),
            }),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            data: serde_json::json!({"data": {"key": "value"}}),
        }
    }

    #[test]
    fn discovery_returns_the_script_and_counts_calls() {
        let snapshot = DiscoverySnapshot {
            resources: vec![discovered_resource(
                "serving.knative.dev",
                "v1",
                "Service",
                "services",
            )],
            warnings: vec!["warning".to_owned()],
        };
        let reader = FakeReader::new().with_discovery(snapshot.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let actual = runtime
            .block_on(reader.discover())
            .expect("scripted discovery");
        assert_eq!(actual, snapshot);
        assert_eq!(reader.discovery_calls(), 1);
    }

    #[test]
    fn discovery_error_is_returned_once() {
        let reader = FakeReader::new().fail_discovery(CoreError::Discovery("no server".to_owned()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let error = runtime
            .block_on(reader.discover())
            .expect_err("scripted discovery error");
        assert!(matches!(error, CoreError::Discovery(_)));
        assert!(runtime.block_on(reader.discover()).is_ok());
    }

    #[test]
    fn list_and_get_are_recorded_and_return_scripted_objects() {
        let first = object("ConfigMap", "default", "alpha");
        let second = object("ConfigMap", "default", "beta");
        let reader = FakeReader::new()
            .with_object(first.clone())
            .with_object(second.clone());
        let ar = dynamic_resource(&Gvk::new("", "v1", "ConfigMap"));
        let gvk = Gvk::new("", "v1", "ConfigMap");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let listed = runtime
            .block_on(reader.list(&ar, Some("default")))
            .expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed
                .iter()
                .map(|object| object.metadata.name.clone().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        let fetched = runtime
            .block_on(reader.get(&gvk, Some("default"), "beta"))
            .expect("get");
        assert_eq!(fetched, second);
        assert_eq!(reader.list_calls().len(), 1);
        assert_eq!(reader.get_calls().len(), 1);
        assert_eq!(reader.list_calls()[0].namespace.as_deref(), Some("default"));
    }

    #[test]
    fn missing_objects_return_not_found() {
        let reader = FakeReader::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let error = runtime
            .block_on(reader.get(&Gvk::new("", "v1", "ConfigMap"), Some("default"), "missing"))
            .expect_err("missing object");
        assert!(matches!(error, CoreError::Kube(_)));
    }

    #[test]
    fn patch_and_dry_run_calls_are_recorded() {
        let patched = object("ConfigMap", "default", "target");
        let reader = FakeReader::new()
            .with_object(patched.clone())
            .with_patch_response(Ok(patched.clone()))
            .with_dry_run_response(Ok(DryRunResult {
                object: patched.clone(),
                warnings: vec!["defaulted".to_owned()],
            }));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let patched_result = runtime
            .block_on(reader.patch_merge(
                &Gvk::new("", "v1", "ConfigMap"),
                "default",
                "target",
                serde_json::json!({"data": {"key": "next"}}),
                true,
            ))
            .expect("patch");
        assert_eq!(patched_result, patched);
        let dry_run = runtime
            .block_on(reader.dry_run_create(patched.clone()))
            .expect("dry run");
        assert_eq!(dry_run.warnings, vec!["defaulted"]);
        assert_eq!(reader.patch_calls().len(), 1);
        assert!(reader.patch_calls()[0].dry_run);
        assert_eq!(reader.dry_run_calls(), vec![patched]);
    }

    #[tokio::test]
    async fn watch_script_is_consumed_once_and_recorded() {
        let object = object("ConfigMap", "default", "watched");
        let event = WatchEvent::Apply(object.clone());
        let reader = FakeReader::new().with_watch(vec![Ok(event)]);
        let ar = dynamic_resource(&Gvk::new("", "v1", "ConfigMap"));
        let mut stream = reader.watch(&ar, Some("default"), CancellationToken::new());
        match stream.next().await {
            Some(Ok(WatchEvent::Apply(actual))) => assert_eq!(actual, object),
            other => panic!("unexpected watch event: {other:?}"),
        }
        assert!(stream.next().await.is_none());
        assert_eq!(reader.watch_calls().len(), 1);
        assert_eq!(
            reader.watch_calls()[0].namespace.as_deref(),
            Some("default")
        );
    }

    #[test]
    fn object_matching_ignores_the_plural() {
        let object = object("ConfigMap", "default", "one");
        let reader = FakeReader::new().with_object(object.clone());
        let ar = ApiResource {
            group: String::new(),
            version: "v1".to_owned(),
            api_version: "v1".to_owned(),
            kind: "ConfigMap".to_owned(),
            plural: "a-different-plural".to_owned(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let listed = runtime
            .block_on(reader.list(&ar, Some("default")))
            .expect("list");
        assert_eq!(listed, vec![object]);
    }
}
