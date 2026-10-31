#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use knect_core::config::DiscoveryConfig;
use knect_core::kube::{DiscoveryManager, KubeReader};
use kube::config::Kubeconfig;

const KUBECONFIG_PATH: &str = "/tmp/knect-e2e-kubeconfig";
const DEFAULT_CONTEXT: &str = "kind-knect-e2e";

fn live_context() -> (String, PathBuf) {
    let path = std::env::var("KUBECONFIG").unwrap_or_else(|_| KUBECONFIG_PATH.to_owned());
    assert_eq!(
        path, KUBECONFIG_PATH,
        "live tests must use the Kind kubeconfig"
    );
    let context =
        std::env::var("KNECT_LIVE_CONTEXT").unwrap_or_else(|_| DEFAULT_CONTEXT.to_owned());
    let kubeconfig = Kubeconfig::read_from(&path).expect("read the Kind kubeconfig");
    assert_eq!(
        kubeconfig.current_context.as_deref(),
        Some(context.as_str()),
        "kubeconfig current context must be the requested live context"
    );
    assert!(
        context.starts_with("kind-"),
        "refusing to run live tests against a non-Kind context"
    );
    (context, PathBuf::from(path))
}

#[tokio::test]
#[ignore = "requires the knect Kind cluster"]
async fn live_discovery_finds_the_expected_operator_groups() {
    let (context, path) = live_context();
    let reader = KubeReader::from_context(Some(&context), Some(&path))
        .await
        .expect("build a reader for the Kind cluster");
    let manager = DiscoveryManager::new(Arc::new(reader), DiscoveryConfig::default());
    let snapshot = manager.discover().await.expect("live discovery");

    let groups: std::collections::BTreeSet<&str> = snapshot
        .resources
        .iter()
        .map(|resource| resource.gvk.group.as_str())
        .collect();
    for expected in [
        "serving.knative.dev",
        "eventing.knative.dev",
        "networking.istio.io",
        "security.istio.io",
        "kyverno.io",
        "flagger.app",
        "kafka.strimzi.io",
        "rabbitmq.com",
        "postgresql.cnpg.io",
        "ceph.rook.io",
        "sources.knative.dev",
    ] {
        assert!(
            groups.contains(expected),
            "discovery did not find {expected}; found {groups:?}"
        );
    }

    let unique: std::collections::BTreeSet<_> = snapshot
        .resources
        .iter()
        .map(|resource| resource.gvk.clone())
        .collect();
    assert_eq!(
        unique.len(),
        snapshot.resources.len(),
        "duplicate GVKs found"
    );
    assert!(
        snapshot
            .resources
            .iter()
            .all(|resource| resource.supports_list_and_watch()),
        "retained resources must support list and watch"
    );
    assert!(
        snapshot
            .resources
            .windows(2)
            .all(|pair| pair[0].gvk <= pair[1].gvk),
        "resources must be sorted by GVK"
    );
    println!(
        "discovered {} watchable resources across {} groups",
        snapshot.resources.len(),
        groups.len()
    );
}
