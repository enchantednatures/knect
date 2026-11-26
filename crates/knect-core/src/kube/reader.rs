//! Production `kube::Client` wrapper implementing [`ClusterReader`].
//!
//! Discovery prefers aggregated discovery and falls back to the legacy
//! per-group discovery flow. Watch and pod-log streaming are backed by the
//! `kube` runtime. No method here contacts a cluster at construction time.

use std::path::Path;

use futures::stream::BoxStream;
use futures::{AsyncBufRead, AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use knect_proto::Gvk;
use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, ListParams, LogParams, Patch, PatchParams,
    PostParams,
};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::discovery::{Discovery, Scope};
use kube::runtime::{WatchStreamExt, watcher};
use tokio_util::sync::CancellationToken;

use super::{
    ClusterReader, CoreFuture, DiscoveredResource, DiscoverySnapshot, DryRunResult, LogOptions,
    WatchEvent,
};
use crate::error::CoreError;

/// Production reader backed by `kube::Client`.
#[derive(Clone)]
pub struct KubeReader {
    client: kube::Client,
}

impl KubeReader {
    /// Wraps an existing client.
    #[must_use]
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Builds a client from the inferred kubeconfig or in-cluster environment.
    pub async fn try_default() -> Result<Self, CoreError> {
        let client = kube::Client::try_default().await.map_err(CoreError::Kube)?;
        Ok(Self::new(client))
    }

    /// Builds a client from an explicit context and optional kubeconfig path.
    ///
    /// When neither a context nor a kubeconfig is given, the configuration is
    /// inferred: the in-cluster service account is preferred, falling back to
    /// the standard kubeconfig search path (including `KUBECONFIG`). With an
    /// explicit context, the standard kubeconfig search path is used. This
    /// never contacts the API server.
    pub async fn from_context(
        context: Option<&str>,
        kubeconfig: Option<&Path>,
    ) -> Result<Self, CoreError> {
        let options = KubeConfigOptions {
            context: context
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            ..KubeConfigOptions::default()
        };
        let config = match kubeconfig {
            Some(path) => {
                let kubeconfig = Kubeconfig::read_from(path).map_err(|error| {
                    CoreError::Config(format!(
                        "cannot read kubeconfig {}: {error}",
                        path.display()
                    ))
                })?;
                kube::Config::from_custom_kubeconfig(kubeconfig, &options)
                    .await
                    .map_err(|error| CoreError::Config(error.to_string()))?
            }
            None if options.context.is_none() => kube::Config::infer()
                .await
                .map_err(|error| CoreError::Config(error.to_string()))?,
            None => kube::Config::from_kubeconfig(&options)
                .await
                .map_err(|error| CoreError::Config(error.to_string()))?,
        };
        let client = kube::Client::try_from(config).map_err(CoreError::Kube)?;
        Ok(Self::new(client))
    }

    /// Returns the underlying client for task-specific extensions.
    #[must_use]
    pub fn client(&self) -> &kube::Client {
        &self.client
    }

    fn api_for(&self, gvk: &Gvk, ns: Option<&str>) -> Api<DynamicObject> {
        let resource = dynamic_resource(gvk);
        match ns {
            Some(namespace) => Api::namespaced_with(self.client.clone(), namespace, &resource),
            None => Api::all_with(self.client.clone(), &resource),
        }
    }
}

impl ClusterReader for KubeReader {
    fn discover(&self) -> CoreFuture<'_, Result<DiscoverySnapshot, CoreError>> {
        let client = self.client.clone();
        Box::pin(async move {
            let discovery = match Discovery::new(client.clone()).run_aggregated().await {
                Ok(discovery) => discovery,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "aggregated discovery unavailable; falling back to legacy discovery"
                    );
                    Discovery::new(client)
                        .run()
                        .await
                        .map_err(CoreError::Kube)?
                }
            };
            let mut resources = Vec::new();
            for group in discovery.groups() {
                for (resource, capabilities) in group.recommended_resources() {
                    resources.push(discovered_resource(
                        resource,
                        capabilities.scope,
                        capabilities.operations,
                    ));
                }
            }
            resources.sort_by(|left, right| left.gvk.cmp(&right.gvk));
            Ok(DiscoverySnapshot {
                resources,
                warnings: Vec::new(),
            })
        })
    }

    fn list<'a>(
        &'a self,
        ar: &'a ApiResource,
        ns: Option<&'a str>,
    ) -> CoreFuture<'a, Result<Vec<DynamicObject>, CoreError>> {
        Box::pin(async move {
            let api = match ns {
                Some(namespace) => Api::namespaced_with(self.client.clone(), namespace, ar),
                None => Api::all_with(self.client.clone(), ar),
            };
            let list = api
                .list(&ListParams::default())
                .await
                .map_err(CoreError::Kube)?;
            Ok(list.items)
        })
    }

    fn watch(
        &self,
        ar: &ApiResource,
        ns: Option<&str>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Result<WatchEvent, CoreError>> {
        let api = match ns {
            Some(namespace) => Api::namespaced_with(self.client.clone(), namespace, ar),
            None => Api::all_with(self.client.clone(), ar),
        };
        watcher::watcher(api, watcher::Config::default().any_semantic())
            .default_backoff()
            .take_until(cancel.cancelled_owned())
            .map(|result| result.map_err(map_watcher_error))
            .boxed()
    }

    fn get<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: Option<&'a str>,
        name: &'a str,
    ) -> CoreFuture<'a, Result<DynamicObject, CoreError>> {
        Box::pin(async move {
            let api = self.api_for(gvk, ns);
            api.get(name).await.map_err(CoreError::Kube)
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
            let resource = dynamic_resource(gvk);
            let api = Api::namespaced_with(self.client.clone(), ns, &resource);
            let params = PatchParams {
                dry_run,
                field_manager: Some("knectd".to_owned()),
                ..PatchParams::default()
            };
            api.patch(name, &params, &Patch::Merge(patch))
                .await
                .map_err(CoreError::Kube)
        })
    }

    fn dry_run_create(
        &self,
        obj: DynamicObject,
    ) -> CoreFuture<'_, Result<DryRunResult, CoreError>> {
        Box::pin(async move {
            let types = obj.types.as_ref().ok_or_else(|| {
                CoreError::Store(
                    "dry-run create requires apiVersion and kind on the object".to_owned(),
                )
            })?;
            let gvk = Gvk::from_api_version(&types.api_version, types.kind.clone());
            let resource = dynamic_resource(&gvk);
            let namespace = obj.metadata.namespace.clone();
            let api = match namespace.as_deref() {
                Some(namespace) => Api::namespaced_with(self.client.clone(), namespace, &resource),
                None => Api::all_with(self.client.clone(), &resource),
            };
            let params = PostParams {
                dry_run: true,
                field_manager: Some("knectd".to_owned()),
            };
            let object = api.create(&params, &obj).await.map_err(CoreError::Kube)?;
            Ok(DryRunResult {
                object,
                warnings: Vec::new(),
            })
        })
    }

    fn create(&self, obj: DynamicObject) -> CoreFuture<'_, Result<DynamicObject, CoreError>> {
        Box::pin(async move {
            let types = obj.types.as_ref().ok_or_else(|| {
                CoreError::Store("create requires apiVersion and kind on the object".to_owned())
            })?;
            let gvk = Gvk::from_api_version(&types.api_version, types.kind.clone());
            let resource = dynamic_resource(&gvk);
            let namespace = obj.metadata.namespace.clone();
            let api = match namespace.as_deref() {
                Some(namespace) => Api::namespaced_with(self.client.clone(), namespace, &resource),
                None => Api::all_with(self.client.clone(), &resource),
            };
            let params = PostParams {
                field_manager: Some("knectd".to_owned()),
                ..PostParams::default()
            };
            api.create(&params, &obj).await.map_err(CoreError::Kube)
        })
    }

    fn delete<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: Option<&'a str>,
        name: &'a str,
    ) -> CoreFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let api = self.api_for(gvk, ns);
            match api.delete(name, &DeleteParams::default()).await {
                Ok(_) => Ok(()),
                Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
                Err(error) => Err(CoreError::Kube(error)),
            }
        })
    }

    fn log_stream(
        &self,
        ns: &str,
        pod: &str,
        opts: LogOptions,
    ) -> BoxStream<'static, Result<String, CoreError>> {
        let client = self.client.clone();
        let namespace = ns.to_owned();
        let pod_name = pod.to_owned();
        futures::stream::unfold(
            LogState::Start {
                client,
                namespace,
                pod_name,
                opts,
            },
            |state| async move {
                let mut state = state;
                loop {
                    match state {
                        LogState::Start {
                            client,
                            namespace,
                            pod_name,
                            opts,
                        } => {
                            let api: Api<Pod> = Api::namespaced(client, &namespace);
                            let container = match resolve_container(&api, &pod_name, &opts).await {
                                Ok(container) => container,
                                Err(error) => {
                                    return Some((Err(error), LogState::Done));
                                }
                            };
                            let params = LogParams {
                                container,
                                follow: opts.follow,
                                tail_lines: opts.tail_lines.map(i64::from),
                                timestamps: opts.timestamps,
                                ..LogParams::default()
                            };
                            match api.log_stream(&pod_name, &params).await {
                                Ok(reader) => {
                                    state = LogState::Reading {
                                        reader: Box::new(reader),
                                    };
                                }
                                Err(error) => {
                                    return Some((Err(CoreError::Kube(error)), LogState::Done));
                                }
                            }
                        }
                        LogState::Reading { mut reader } => {
                            let mut line = String::new();
                            match reader.read_line(&mut line).await {
                                Ok(0) => return None,
                                Ok(_) => {
                                    let line = line.trim_end_matches(['\r', '\n']).to_owned();
                                    return Some((Ok(line), LogState::Reading { reader }));
                                }
                                Err(error) => {
                                    return Some((
                                        Err(CoreError::Internal(format!(
                                            "pod log stream read failed: {error}"
                                        ))),
                                        LogState::Done,
                                    ));
                                }
                            }
                        }
                        LogState::Done => return None,
                    }
                }
            },
        )
        .boxed()
    }
}

enum LogState {
    Start {
        client: kube::Client,
        namespace: String,
        pod_name: String,
        opts: LogOptions,
    },
    Reading {
        reader: Box<dyn AsyncBufRead + Send + Unpin>,
    },
    Done,
}

async fn resolve_container(
    api: &Api<Pod>,
    pod_name: &str,
    opts: &LogOptions,
) -> Result<Option<String>, CoreError> {
    if let Some(container) = &opts.container {
        return Ok(Some(container.clone()));
    }
    let pod = api.get(pod_name).await.map_err(CoreError::Kube)?;
    if let Some(annotations) = &pod.metadata.annotations
        && let Some(container) = annotations
            .get("kubectl.kubernetes.io/default-container")
            .cloned()
    {
        return Ok(Some(container));
    }
    Ok(pod.spec.and_then(|spec| {
        spec.containers
            .first()
            .map(|container| container.name.clone())
    }))
}

/// Builds a dynamic API resource, guessing the plural.
///
/// Watchers use the [`ApiResource`] supplied by discovery, so their plural is
/// exact. `get`/`patch_merge` still synthesize an [`ApiResource`] from a GVK;
/// a later reader cache can replace the guess with the discovered plural.
#[must_use]
pub fn dynamic_resource(gvk: &Gvk) -> ApiResource {
    ApiResource {
        group: gvk.group.clone(),
        version: gvk.version.clone(),
        api_version: gvk.api_version(),
        kind: gvk.kind.clone(),
        plural: guess_plural(&gvk.kind),
    }
}

fn discovered_resource(
    resource: ApiResource,
    scope: Scope,
    operations: Vec<String>,
) -> DiscoveredResource {
    DiscoveredResource {
        gvk: Gvk::new(
            resource.group.clone(),
            resource.version.clone(),
            resource.kind.clone(),
        ),
        plural: resource.plural,
        api_version: resource.api_version,
        namespaced: scope == Scope::Namespaced,
        verbs: operations,
    }
}

fn map_watcher_error(error: watcher::Error) -> CoreError {
    match error {
        watcher::Error::InitialListFailed(source)
        | watcher::Error::WatchStartFailed(source)
        | watcher::Error::WatchFailed(source) => map_client_error(source),
        watcher::Error::WatchError(status) => CoreError::Kube(kube::Error::Api(status)),
        watcher::Error::NoResourceVersion => {
            CoreError::Watch("watch result has no metadata.resourceVersion".to_owned())
        }
    }
}

fn map_client_error(error: kube::Error) -> CoreError {
    match error {
        kube::Error::Api(status) => CoreError::Kube(kube::Error::Api(status)),
        other => CoreError::Watch(other.to_string()),
    }
}

/// Naive pluralization used only until discovery supplies the real plural.
#[must_use]
pub fn guess_plural(kind: &str) -> String {
    let lower = kind.to_ascii_lowercase();
    if lower.ends_with('s')
        || lower.ends_with('x')
        || lower.ends_with("ch")
        || lower.ends_with("sh")
    {
        format!("{lower}es")
    } else if let Some(stem) = lower.strip_suffix('y') {
        format!("{stem}ies")
    } else {
        format!("{lower}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::ResourceExt;

    #[test]
    fn dynamic_resource_keeps_api_version_and_guesses_plural() {
        let gvk = Gvk::new("serving.knative.dev", "v1", "Service");
        let resource = dynamic_resource(&gvk);
        assert_eq!(resource.group, "serving.knative.dev");
        assert_eq!(resource.version, "v1");
        assert_eq!(resource.api_version, "serving.knative.dev/v1");
        assert_eq!(resource.kind, "Service");
        assert_eq!(resource.plural, "services");

        let core = dynamic_resource(&Gvk::new("", "v1", "Ingress"));
        assert_eq!(core.api_version, "v1");
        assert_eq!(core.plural, "ingresses");
    }

    #[test]
    fn plural_guesses_cover_common_suffixes() {
        assert_eq!(guess_plural("Pod"), "pods");
        assert_eq!(guess_plural("Policy"), "policies");
        assert_eq!(guess_plural("Ingress"), "ingresses");
        assert_eq!(guess_plural("Match"), "matches");
    }

    #[test]
    fn dry_run_result_carries_object_and_warnings() {
        let gvk = Gvk::new("v1", "v1", "ConfigMap");
        let mut object = DynamicObject::new("example", &dynamic_resource(&gvk));
        object.data = serde_json::json!({"data": {"key": "value"}});
        let result = DryRunResult {
            object,
            warnings: vec!["defaulted".to_owned()],
        };
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.object.name_any(), "example");
    }

    #[test]
    fn reader_construction_type_checks() {
        fn assert_reader<T: ClusterReader>() {}
        assert_reader::<KubeReader>();
    }
}
