//! Discovery filtering and the manager used by the daemon.
//!
//! `KubeReader::discover` returns every recommended resource the API server
//! exposes. [`DiscoveryManager`] narrows that set to the configured groups and
//! to resources that can actually be listed and watched, then records a
//! deterministic warning for every configured group that is absent.

use std::collections::BTreeSet;
use std::sync::Arc;

use knect_proto::Gvk;

use super::{ClusterReader, CoreFuture, DiscoveredResource, DiscoverySnapshot};
use crate::config::DiscoveryConfig;
use crate::error::CoreError;

/// Filters discovery results to watchable resources in configured groups.
#[derive(Clone)]
pub struct DiscoveryManager {
    reader: Arc<dyn ClusterReader>,
    config: DiscoveryConfig,
}

impl DiscoveryManager {
    /// Creates a manager over a shared reader and discovery configuration.
    #[must_use]
    pub fn new(reader: Arc<dyn ClusterReader>, config: DiscoveryConfig) -> Self {
        Self { reader, config }
    }

    /// Returns the shared reader.
    #[must_use]
    pub fn reader(&self) -> &Arc<dyn ClusterReader> {
        &self.reader
    }

    /// Returns the discovery configuration.
    #[must_use]
    pub fn config(&self) -> &DiscoveryConfig {
        &self.config
    }

    /// Runs discovery and returns the filtered, deterministic snapshot.
    pub fn discover(&self) -> CoreFuture<'_, Result<DiscoverySnapshot, CoreError>> {
        Box::pin(async move {
            let snapshot = self.reader.discover().await?;
            Ok(filter_snapshot(snapshot, &self.config))
        })
    }
}

/// Applies group and verb filters, deduplicates GVKs, and adds warnings.
///
/// The reader already selects the recommended version of each kind; this
/// function is deliberately pure so the selection rules are unit-testable.
#[must_use]
pub fn filter_snapshot(snapshot: DiscoverySnapshot, config: &DiscoveryConfig) -> DiscoverySnapshot {
    let allow_all = config.groups.is_empty();
    let mut resources = snapshot.resources;
    resources.retain(|resource| {
        (allow_all || group_is_allowed(&config.groups, &resource.gvk.group))
            && resource.supports_list_and_watch()
            && (config.secrets || !is_secret(&resource.gvk))
    });
    resources.sort_by(compare_resources);
    resources.dedup_by(|left, right| left.gvk == right.gvk);

    let present: BTreeSet<&str> = resources
        .iter()
        .map(|resource| resource.gvk.group.as_str())
        .collect();
    let mut warnings = snapshot.warnings;
    for group in &config.groups {
        if !present.contains(group.as_str()) {
            warnings.push(format!("group {group} is not installed"));
        }
    }
    warnings.sort();
    warnings.dedup();

    DiscoverySnapshot {
        resources,
        warnings,
    }
}

/// Finds a discovered resource by GVK.
#[must_use]
pub fn find<'a>(snapshot: &'a DiscoverySnapshot, gvk: &Gvk) -> Option<&'a DiscoveredResource> {
    snapshot.get(gvk)
}

fn group_is_allowed(groups: &[String], group: &str) -> bool {
    groups.iter().any(|allowed| allowed == group)
}

fn is_secret(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && gvk.kind == "Secret"
}

fn compare_resources(left: &DiscoveredResource, right: &DiscoveredResource) -> std::cmp::Ordering {
    left.gvk
        .cmp(&right.gvk)
        .then_with(|| left.api_version.cmp(&right.api_version))
        .then_with(|| left.plural.cmp(&right.plural))
        .then_with(|| left.verbs.cmp(&right.verbs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(group: &str, version: &str, kind: &str, verbs: &[&str]) -> DiscoveredResource {
        DiscoveredResource {
            gvk: Gvk::new(group, version, kind),
            plural: format!("{}s", kind.to_ascii_lowercase()),
            api_version: Gvk::new(group, version, kind).api_version(),
            namespaced: true,
            verbs: verbs.iter().map(|verb| (*verb).to_owned()).collect(),
        }
    }

    fn snapshot(resources: Vec<DiscoveredResource>) -> DiscoverySnapshot {
        DiscoverySnapshot {
            resources,
            warnings: Vec::new(),
        }
    }

    fn config(groups: &[&str]) -> DiscoveryConfig {
        DiscoveryConfig {
            groups: groups.iter().map(|group| (*group).to_owned()).collect(),
            ..DiscoveryConfig::default()
        }
    }

    #[test]
    fn allowlist_keeps_only_configured_groups() {
        let input = snapshot(vec![
            resource("serving.knative.dev", "v1", "Service", &["list", "watch"]),
            resource("eventing.knative.dev", "v1", "Broker", &["list", "watch"]),
            resource("", "v1", "ConfigMap", &["list", "watch"]),
        ]);
        let filtered = filter_snapshot(input, &config(&["eventing.knative.dev"]));
        assert_eq!(filtered.resources.len(), 1);
        assert_eq!(filtered.resources[0].gvk.group, "eventing.knative.dev");
        assert!(filtered.warnings.is_empty());
    }

    #[test]
    fn core_secrets_are_excluded_unless_explicitly_enabled() {
        let input = snapshot(vec![
            resource("", "v1", "Pod", &["list", "watch"]),
            resource("", "v1", "Secret", &["list", "watch"]),
        ]);
        let filtered = filter_snapshot(input.clone(), &config(&[""]));
        assert_eq!(filtered.resources.len(), 1);
        assert_eq!(filtered.resources[0].gvk.kind, "Pod");

        let mut secret_config = config(&[""]);
        secret_config.secrets = true;
        let filtered = filter_snapshot(input, &secret_config);
        assert_eq!(filtered.resources.len(), 2);
        assert!(
            filtered
                .resources
                .iter()
                .any(|resource| resource.gvk.kind == "Secret")
        );
    }

    #[test]
    fn empty_allowlist_keeps_every_group() {
        let input = snapshot(vec![
            resource("serving.knative.dev", "v1", "Service", &["list", "watch"]),
            resource("", "v1", "Pod", &["list", "watch"]),
        ]);
        let filtered = filter_snapshot(input, &config(&[]));
        assert_eq!(filtered.resources.len(), 2);
        assert!(filtered.warnings.is_empty());
    }

    #[test]
    fn list_and_watch_verbs_are_both_required() {
        let input = snapshot(vec![
            resource("serving.knative.dev", "v1", "Service", &["get", "list"]),
            resource("serving.knative.dev", "v1", "Revision", &["list", "watch"]),
            resource("serving.knative.dev", "v1", "Route", &["watch"]),
        ]);
        let filtered = filter_snapshot(input, &config(&["serving.knative.dev"]));
        assert_eq!(filtered.resources.len(), 1);
        assert_eq!(filtered.resources[0].gvk.kind, "Revision");
    }

    #[test]
    fn verbs_are_case_sensitive() {
        let input = snapshot(vec![resource(
            "serving.knative.dev",
            "v1",
            "Service",
            &["LIST", "WATCH"],
        )]);
        let filtered = filter_snapshot(input, &config(&["serving.knative.dev"]));
        assert!(filtered.resources.is_empty());
        assert_eq!(
            filtered.warnings,
            vec!["group serving.knative.dev is not installed"]
        );
    }

    #[test]
    fn every_missing_group_produces_a_sorted_warning() {
        let input = snapshot(vec![resource(
            "serving.knative.dev",
            "v1",
            "Service",
            &["list", "watch"],
        )]);
        let filtered = filter_snapshot(
            input,
            &config(&[
                "serving.knative.dev",
                "kyverno.io",
                "ceph.rook.io",
                "flagger.app",
            ]),
        );
        assert_eq!(
            filtered.warnings,
            vec![
                "group ceph.rook.io is not installed",
                "group flagger.app is not installed",
                "group kyverno.io is not installed",
            ]
        );
    }

    #[test]
    fn duplicate_gvks_keep_one_deterministic_entry() {
        let mut first = resource("serving.knative.dev", "v1", "Service", &["list", "watch"]);
        first.plural = "zeta".to_owned();
        let mut second = resource("serving.knative.dev", "v1", "Service", &["list", "watch"]);
        second.plural = "alpha".to_owned();
        let filtered = filter_snapshot(
            snapshot(vec![first, second]),
            &config(&["serving.knative.dev"]),
        );
        assert_eq!(filtered.resources.len(), 1);
        assert_eq!(filtered.resources[0].plural, "alpha");
    }

    #[test]
    fn output_order_is_deterministic() {
        let first = resource("serving.knative.dev", "v1", "Service", &["list", "watch"]);
        let second = resource("eventing.knative.dev", "v1", "Broker", &["list", "watch"]);
        let forward = filter_snapshot(snapshot(vec![first.clone(), second.clone()]), &config(&[]));
        let reverse = filter_snapshot(snapshot(vec![second, first]), &config(&[]));
        assert_eq!(forward, reverse);
        assert_eq!(forward.resources[0].gvk.group, "eventing.knative.dev");
    }

    #[test]
    fn find_matches_the_gvk_exactly() {
        let gvk = Gvk::new("serving.knative.dev", "v1", "Service");
        let input = snapshot(vec![resource(
            "serving.knative.dev",
            "v1",
            "Service",
            &["list", "watch"],
        )]);
        assert!(find(&input, &gvk).is_some());
        assert!(
            find(
                &input,
                &Gvk::new("serving.knative.dev", "v1beta1", "Service")
            )
            .is_none()
        );
    }

    #[test]
    fn api_resource_uses_discovered_plural() {
        let resource = resource("kyverno.io", "v1", "ClusterPolicy", &["list", "watch"]);
        let ar = resource.api_resource();
        assert_eq!(ar.group, "kyverno.io");
        assert_eq!(ar.version, "v1");
        assert_eq!(ar.api_version, "kyverno.io/v1");
        assert_eq!(ar.kind, "ClusterPolicy");
        assert_eq!(ar.plural, "clusterpolicys");
    }

    #[test]
    fn discovery_manager_filters_the_reader_result() {
        use crate::kube::FakeReader;

        let reader = Arc::new(FakeReader::new().with_discovery(DiscoverySnapshot {
            resources: vec![
                resource("serving.knative.dev", "v1", "Service", &["list", "watch"]),
                resource("kyverno.io", "v1", "Policy", &["get"]),
            ],
            warnings: vec!["reader warning".to_owned()],
        }));
        let manager = DiscoveryManager::new(reader, config(&["serving.knative.dev"]));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let snapshot = runtime.block_on(manager.discover()).expect("discovery");
        assert_eq!(snapshot.resources.len(), 1);
        assert_eq!(snapshot.warnings, vec!["reader warning"]);
    }
}
