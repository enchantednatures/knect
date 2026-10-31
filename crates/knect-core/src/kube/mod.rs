//! Kubernetes access boundary.
//!
//! Correlators and the daemon depend on the [`ClusterReader`] trait, never on
//! `kube` directly. Discovery, watch supervision, sanitization, and the fake
//! reader live in sibling modules; the trait shape is frozen here.
//!
//! The trait is hand-desugared instead of using `async_trait` because the
//! workspace dependency pin does not include `async-trait`. The desugared form
//! is object-safe (`Arc<dyn ClusterReader>`), which the async-trait attribute
//! would also provide.

pub mod discovery;
pub mod fake;
pub mod reader;
pub mod sanitize;
pub mod watch;

use std::future::Future;
use std::pin::Pin;

use futures::stream::BoxStream;
use knect_proto::Gvk;
use kube::api::{ApiResource, DynamicObject};
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;

pub use discovery::DiscoveryManager;
pub use fake::FakeReader;
pub use reader::{KubeReader, guess_plural};
pub use sanitize::{SanitizedObject, Sanitizer, SanitizerConfig, SecretMaterial};
pub use watch::{WatchHandle, WatchManager, WatchOutcome, WatchUpdate};

/// A boxed, sendable future used by the object-safe reader trait.
pub type CoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A Kubernetes watch event for a dynamic object.
///
/// Task 3 moved this alias from the legacy `kube::api::WatchEvent` to the
/// runtime watcher event so supervisors can observe `Init`, `InitApply`,
/// `Apply`, `Delete`, and `InitDone`. The event variants are unchanged for
/// consumers that only match `Apply`/`Delete`.
pub type WatchEvent = kube::runtime::watcher::Event<DynamicObject>;

/// One discovered API resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredResource {
    /// Group, version, and kind.
    pub gvk: Gvk,
    /// Lower-case plural resource name.
    pub plural: String,
    /// Canonical `apiVersion` string.
    pub api_version: String,
    /// True when the resource is namespace-scoped.
    pub namespaced: bool,
    /// Verbs advertised by the API server.
    pub verbs: Vec<String>,
}

impl DiscoveredResource {
    /// Returns the `kube` API resource descriptor for this discovery result.
    ///
    /// Builds the descriptor from discovered values only; it never guesses the
    /// plural because discovery already supplied it.
    #[must_use]
    pub fn api_resource(&self) -> ApiResource {
        ApiResource {
            group: self.gvk.group.clone(),
            version: self.gvk.version.clone(),
            api_version: self.api_version.clone(),
            kind: self.gvk.kind.clone(),
            plural: self.plural.clone(),
        }
    }

    /// Returns true when the API server advertises both `list` and `watch`.
    #[must_use]
    pub fn supports_list_and_watch(&self) -> bool {
        has_verb(&self.verbs, "list") && has_verb(&self.verbs, "watch")
    }
}

fn has_verb(verbs: &[String], expected: &str) -> bool {
    verbs.iter().any(|verb| verb == expected)
}

/// Result of an API discovery pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoverySnapshot {
    /// Recommended resources, sorted by GVK.
    pub resources: Vec<DiscoveredResource>,
    /// Non-fatal discovery warnings.
    pub warnings: Vec<String>,
}

impl DiscoverySnapshot {
    /// Returns true when no resources were discovered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    /// Finds a resource by GVK.
    #[must_use]
    pub fn get(&self, gvk: &Gvk) -> Option<&DiscoveredResource> {
        self.resources.iter().find(|resource| &resource.gvk == gvk)
    }
}

/// Result of a server-side dry-run create.
#[derive(Debug, Clone, PartialEq)]
pub struct DryRunResult {
    /// The object returned by the API server, including admission mutations.
    pub object: DynamicObject,
    /// Non-fatal warnings produced while applying the object.
    pub warnings: Vec<String>,
}

/// Options for pod log streaming.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogOptions {
    /// Container name. `None` selects the pod default.
    pub container: Option<String>,
    /// Number of historical lines to return first.
    pub tail_lines: Option<u32>,
    /// Include kubelet timestamps.
    pub timestamps: bool,
    /// Keep the stream open for new lines.
    pub follow: bool,
}

/// Read and write access to a Kubernetes cluster.
///
/// Implementations must be `Send + Sync` and safe to share across tasks.
#[allow(clippy::type_complexity)]
pub trait ClusterReader: Send + Sync {
    /// Runs API discovery and returns the recommended resource set.
    fn discover(&self) -> CoreFuture<'_, Result<DiscoverySnapshot, CoreError>>;

    /// Lists objects for one API resource.
    fn list<'a>(
        &'a self,
        ar: &'a ApiResource,
        ns: Option<&'a str>,
    ) -> CoreFuture<'a, Result<Vec<DynamicObject>, CoreError>>;

    /// Watches objects for one API resource.
    ///
    /// The stream ends after `cancel` fires. Errors are reported as values so
    /// a supervisor can classify transient failures and CRD deletion (`404`).
    fn watch(
        &self,
        ar: &ApiResource,
        ns: Option<&str>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Result<WatchEvent, CoreError>>;

    /// Gets one object.
    fn get<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: Option<&'a str>,
        name: &'a str,
    ) -> CoreFuture<'a, Result<DynamicObject, CoreError>>;

    /// Applies a JSON merge patch, optionally as a server-side dry run.
    fn patch_merge<'a>(
        &'a self,
        gvk: &'a Gvk,
        ns: &'a str,
        name: &'a str,
        patch: serde_json::Value,
        dry_run: bool,
    ) -> CoreFuture<'a, Result<DynamicObject, CoreError>>;

    /// Creates an object with `dryRun=All` and returns admission mutations.
    fn dry_run_create(&self, obj: DynamicObject)
    -> CoreFuture<'_, Result<DryRunResult, CoreError>>;

    /// Streams pod logs.
    ///
    /// The foundation implementation yields exactly one
    /// [`CoreError::Unsupported`]; Task 12 replaces it with a real stream.
    fn log_stream(
        &self,
        ns: &str,
        pod: &str,
        opts: LogOptions,
    ) -> BoxStream<'static, Result<String, CoreError>>;
}

/// Compile-time proof that the reader trait is usable behind a trait object.
#[must_use]
pub fn as_dyn(reader: std::sync::Arc<dyn ClusterReader>) -> std::sync::Arc<dyn ClusterReader> {
    reader
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_snapshot_lookup_is_stable() {
        let gvk = Gvk::new("serving.knative.dev", "v1", "Service");
        let snapshot = DiscoverySnapshot {
            resources: vec![DiscoveredResource {
                gvk: gvk.clone(),
                plural: "services".to_owned(),
                api_version: "serving.knative.dev/v1".to_owned(),
                namespaced: true,
                verbs: vec!["get".to_owned(), "list".to_owned()],
            }],
            warnings: vec!["missing eventing group".to_owned()],
        };
        assert!(!snapshot.is_empty());
        assert_eq!(
            snapshot.get(&gvk).map(|r| r.plural.as_str()),
            Some("services")
        );
        assert!(snapshot.get(&Gvk::new("v1", "v1", "Pod")).is_none());
    }

    #[test]
    fn reader_trait_is_object_safe_and_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ClusterReader>();
        let _function: fn(std::sync::Arc<dyn ClusterReader>) -> std::sync::Arc<dyn ClusterReader> =
            as_dyn;
    }

    #[test]
    fn log_options_default_is_quiet() {
        let options = LogOptions::default();
        assert!(options.container.is_none());
        assert_eq!(options.tail_lines, None);
        assert!(!options.timestamps);
        assert!(!options.follow);
    }
}
