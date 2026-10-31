#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use knect_core::config::WatchConfig;
use knect_core::kube::fake::not_found_error;
use knect_core::kube::watch::{BackoffPolicy, WatchOutcome, WatchUpdate};
use knect_core::kube::{DiscoveredResource, FakeReader, WatchEvent, WatchManager};
use knect_proto::Gvk;
use kube::api::{ApiResource, DynamicObject};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn discovered(group: &str, version: &str, kind: &str, plural: &str) -> DiscoveredResource {
    DiscoveredResource {
        gvk: Gvk::new(group, version, kind),
        plural: plural.to_owned(),
        api_version: Gvk::new(group, version, kind).api_version(),
        namespaced: true,
        verbs: vec!["list".to_owned(), "watch".to_owned()],
    }
}

fn object(kind: &str, name: &str) -> DynamicObject {
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
    .within("default")
}

fn manager(reader: FakeReader) -> WatchManager {
    WatchManager::with_backoff(
        Arc::new(reader),
        WatchConfig::default(),
        BackoffPolicy::new(Duration::from_millis(1), Duration::from_millis(2), 4),
    )
}

#[tokio::test]
async fn init_apply_delete_events_map_to_typed_updates() {
    let reader = FakeReader::new().with_watch(vec![
        Ok(WatchEvent::Init),
        Ok(WatchEvent::InitApply(object("ConfigMap", "one"))),
        Ok(WatchEvent::Apply(object("ConfigMap", "one"))),
        Ok(WatchEvent::Delete(object("ConfigMap", "one"))),
        Ok(WatchEvent::InitDone),
    ]);
    let manager = manager(reader);
    let cancel = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel(16);
    let handles = manager.spawn_with_updates(
        &[discovered("v1", "v1", "ConfigMap", "configmaps")],
        cancel.clone(),
        sender,
    );

    let mut seen = Vec::new();
    while seen.len() < 5 {
        let update = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("update arrives")
            .expect("update channel is open");
        seen.push(update);
    }
    assert!(matches!(seen[0], WatchUpdate::Init { .. }));
    assert!(matches!(seen[1], WatchUpdate::InitApply { .. }));
    assert!(matches!(seen[2], WatchUpdate::Apply { .. }));
    assert!(matches!(seen[3], WatchUpdate::Delete { .. }));
    assert!(matches!(seen[4], WatchUpdate::InitDone { .. }));

    cancel.cancel();
    for handle in handles {
        let outcome = tokio::time::timeout(Duration::from_secs(2), handle.task)
            .await
            .expect("watcher stops")
            .expect("watcher task");
        assert!(matches!(outcome, WatchOutcome::Cancelled { .. }));
    }
}

#[tokio::test]
async fn crd_404_stops_the_watcher_for_rediscovery() {
    let reader = FakeReader::new().with_watch(vec![Err(not_found_error("widgets.example.com"))]);
    let manager = manager(reader);
    let handles = manager.spawn(
        &[discovered("example.com", "v1", "Widget", "widgets")],
        CancellationToken::new(),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        handles.into_iter().next().unwrap().task,
    )
    .await
    .expect("watcher stops")
    .expect("watcher task");
    assert!(matches!(outcome, WatchOutcome::ResourceGone { .. }));
}

#[tokio::test]
async fn cancellation_stops_all_watchers() {
    let reader = FakeReader::new();
    let manager = manager(reader);
    let cancel = CancellationToken::new();
    let handles = manager.spawn(
        &[
            discovered("v1", "v1", "ConfigMap", "configmaps"),
            discovered("v1", "v1", "Secret", "secrets"),
        ],
        cancel.clone(),
    );
    assert_eq!(handles.len(), 2);
    cancel.cancel();
    for handle in handles {
        let outcome = tokio::time::timeout(Duration::from_secs(2), handle.task)
            .await
            .expect("watcher stops")
            .expect("watcher task");
        assert!(matches!(outcome, WatchOutcome::Cancelled { .. }));
    }
}

#[test]
fn backoff_schedule_is_deterministic_and_bounded() {
    let policy = BackoffPolicy::new(Duration::from_millis(200), Duration::from_secs(30), 6);
    assert_eq!(
        policy.schedule(),
        vec![
            Duration::from_millis(200),
            Duration::from_millis(400),
            Duration::from_millis(800),
            Duration::from_millis(1_600),
            Duration::from_millis(3_200),
            Duration::from_millis(6_400),
        ]
    );
    let capped = BackoffPolicy::new(Duration::from_millis(200), Duration::from_secs(30), 10);
    assert_eq!(
        capped.schedule().last().copied(),
        Some(Duration::from_secs(30))
    );
    assert!(capped.delay(11).is_none());
}
