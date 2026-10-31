#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use knect_core::kube::{ClusterReader, KubeReader};
use knect_proto::Gvk;
use kube::api::ApiResource;
use kube::config::Kubeconfig;
use tokio_util::sync::CancellationToken;

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
async fn live_watch_streams_configmaps_and_stops_on_cancel() {
    let (context, path) = live_context();
    let reader = KubeReader::from_context(Some(&context), Some(&path))
        .await
        .expect("build a reader for the Kind cluster");
    let config_map = ApiResource {
        group: String::new(),
        version: "v1".to_owned(),
        api_version: "v1".to_owned(),
        kind: "ConfigMap".to_owned(),
        plural: "configmaps".to_owned(),
    };
    let gvk = Gvk::new("", "v1", "ConfigMap");
    assert_eq!(gvk.api_version(), config_map.api_version);

    let cancel = CancellationToken::new();
    let mut stream = reader.watch(&config_map, Some("knect-e2e"), cancel.clone());
    let consumed = tokio::time::timeout(Duration::from_secs(2), async {
        let mut events = 0_usize;
        while let Some(item) = stream.next().await {
            match item {
                Ok(_) => events = events.saturating_add(1),
                Err(error) => {
                    assert!(
                        !matches!(error, knect_core::CoreError::Unsupported(_)),
                        "watch returned an unsupported error: {error}"
                    );
                    println!("transient watch error while consuming: {error}");
                }
            }
        }
        events
    })
    .await;
    match consumed {
        Ok(events) => println!("watch stream ended after {events} events"),
        Err(_) => println!("consumed the watch stream for two seconds"),
    }

    cancel.cancel();
    let stopped = tokio::time::timeout(Duration::from_secs(5), async {
        while stream.next().await.is_some() {}
    })
    .await;
    assert!(
        stopped.is_ok(),
        "watch stream did not stop after cancellation"
    );
    println!("watch stream stopped cleanly after cancellation");
}
