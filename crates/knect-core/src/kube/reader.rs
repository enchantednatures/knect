//! Production `kube::Client` wrapper implementing [`ClusterReader`].
//!
//! Discovery works through `kube::discovery::Discovery`. Watch and log
//! streaming return a single typed [`CoreError::Unsupported`] until Tasks 3 and
//! 12 install the real implementations. No method here contacts a cluster at
//! construction time.

use std::path::Path;

use futures::StreamExt;
use futures::stream::BoxStream;
use knect_proto::Gvk;
use kube::api::{Api, ApiResource, DynamicObject, ListParams, Patch, PatchParams, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::discovery::{Discovery, Scope};
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
    /// When `kubeconfig` is `None`, the standard kubeconfig search path is
    /// used (including `KUBECONFIG`). This never contacts the API server.
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
            let discovery = Discovery::new(client)
                .run()
                .await
                .map_err(CoreError::Kube)?;
            let mut resources = Vec::new();
            for group in discovery.groups() {
                for (resource, capabilities) in group.recommended_resources() {
                    resources.push(DiscoveredResource {
                        gvk: Gvk::new(
                            resource.group.clone(),
                            resource.version.clone(),
                            resource.kind.clone(),
                        ),
                        plural: resource.plural,
                        api_version: resource.api_version,
                        namespaced: capabilities.scope == Scope::Namespaced,
                        verbs: capabilities.operations,
                    });
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
        _ar: &ApiResource,
        _ns: Option<&str>,
        _cancel: CancellationToken,
    ) -> BoxStream<'static, Result<WatchEvent, CoreError>> {
        futures::stream::once(async {
            Err(CoreError::Unsupported(
                "watch streaming is implemented by Task 3".to_owned(),
            ))
        })
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

    fn log_stream(
        &self,
        _ns: &str,
        _pod: &str,
        _opts: LogOptions,
    ) -> BoxStream<'static, Result<String, CoreError>> {
        futures::stream::once(async {
            Err(CoreError::Unsupported(
                "log streaming is implemented by Task 12".to_owned(),
            ))
        })
        .boxed()
    }
}

/// Builds a dynamic API resource, guessing the plural.
///
/// Task 3 replaces the guess with the discovered plural. The guess exists so
/// Tasks 5-12 can call `get`/`patch` before discovery is wired.
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
