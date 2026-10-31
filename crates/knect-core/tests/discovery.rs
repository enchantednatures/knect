#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use knect_core::config::DiscoveryConfig;
use knect_core::kube::discovery::{filter_snapshot, find};
use knect_core::kube::{DiscoveredResource, DiscoveryManager, DiscoverySnapshot, FakeReader};
use knect_proto::Gvk;

fn resource(group: &str, kind: &str, verbs: &[&str]) -> DiscoveredResource {
    DiscoveredResource {
        gvk: Gvk::new(group, "v1", kind),
        plural: format!("{}s", kind.to_ascii_lowercase()),
        api_version: format!("{group}/v1"),
        namespaced: true,
        verbs: verbs.iter().map(|verb| (*verb).to_owned()).collect(),
    }
}

#[test]
fn manager_filters_groups_and_verbs_and_warns_for_missing_groups() {
    let snapshot = DiscoverySnapshot {
        resources: vec![
            resource("serving.knative.dev", "Service", &["list", "watch"]),
            resource("eventing.knative.dev", "Broker", &["list", "watch"]),
            resource("kyverno.io", "Policy", &["get", "list"]),
        ],
        warnings: Vec::new(),
    };
    let reader = Arc::new(FakeReader::new().with_discovery(snapshot));
    let config = DiscoveryConfig {
        groups: vec![
            "serving.knative.dev".to_owned(),
            "kyverno.io".to_owned(),
            "flagger.app".to_owned(),
        ],
        ..DiscoveryConfig::default()
    };
    let manager = DiscoveryManager::new(reader, config);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let result = runtime.block_on(manager.discover()).expect("discovery");

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources[0].gvk.group, "serving.knative.dev");
    assert_eq!(
        result.warnings,
        vec![
            "group flagger.app is not installed",
            "group kyverno.io is not installed",
        ]
    );
    assert!(find(&result, &Gvk::new("serving.knative.dev", "v1", "Service")).is_some());
}

#[test]
fn filter_is_deterministic_and_deduplicates_gvks() {
    let mut first = resource("serving.knative.dev", "Service", &["list", "watch"]);
    first.plural = "services-z".to_owned();
    let mut duplicate = resource("serving.knative.dev", "Service", &["list", "watch"]);
    duplicate.plural = "services-a".to_owned();
    let snapshot = DiscoverySnapshot {
        resources: vec![duplicate, first],
        warnings: Vec::new(),
    };
    let filtered = filter_snapshot(
        snapshot,
        &DiscoveryConfig {
            groups: vec!["serving.knative.dev".to_owned()],
            ..DiscoveryConfig::default()
        },
    );
    assert_eq!(filtered.resources.len(), 1);
    assert_eq!(filtered.resources[0].plural, "services-a");
}

#[test]
fn empty_group_allowlist_keeps_all_watchable_resources() {
    let snapshot = DiscoverySnapshot {
        resources: vec![
            resource("serving.knative.dev", "Service", &["list", "watch"]),
            resource("", "ConfigMap", &["list", "watch"]),
            resource("eventing.knative.dev", "Broker", &["list"]),
        ],
        warnings: Vec::new(),
    };
    let filtered = filter_snapshot(
        snapshot,
        &DiscoveryConfig {
            groups: Vec::new(),
            ..DiscoveryConfig::default()
        },
    );
    assert_eq!(filtered.resources.len(), 2);
    assert!(filtered.warnings.is_empty());
}
