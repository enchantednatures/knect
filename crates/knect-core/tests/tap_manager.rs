#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Trigger-backed tap lifecycle, ring, TTL, GC, and scripted CloudEvent tests.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use knect_core::kube::fake::FakeReader;
use knect_core::tap::{
    TapCallbackRequest, TapDelivery, TapManager, TapManagerConfig, TriggerTapManager,
};
use knect_proto::rpc::TapStartParams;
use knect_proto::tap::{TapFilter, TapHandle, TapState, TapTarget, TapTargetKind};
use kube::api::{ApiResource, DynamicObject};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures/tap")
        .join(name);
    let raw = std::fs::read_to_string(path).expect("tap fixture is readable");
    serde_json::from_str(&raw).expect("tap fixture parses")
}

fn resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: group.to_owned(),
        version: version.to_owned(),
        api_version: if group.is_empty() {
            version.to_owned()
        } else {
            format!("{group}/{version}")
        },
        kind: kind.to_owned(),
        plural: plural.to_owned(),
    }
}

fn object(resource: &ApiResource, namespace: &str, name: &str) -> DynamicObject {
    let mut object = DynamicObject::new(name, resource);
    object.metadata.namespace = Some(namespace.to_owned());
    object
}

fn broker(namespace: &str, name: &str) -> DynamicObject {
    let mut broker = object(
        &resource("eventing.knative.dev", "v1", "Broker", "brokers"),
        namespace,
        name,
    );
    broker.data = json!({"spec": {}});
    broker
}

fn managed_trigger(namespace: &str, id: &str) -> DynamicObject {
    let mut trigger = object(
        &resource("eventing.knative.dev", "v1", "Trigger", "triggers"),
        namespace,
        id,
    );
    trigger.metadata.labels = Some(BTreeMap::from([
        ("knect.dev/managed-by".to_owned(), "knect".to_owned()),
        ("knect.dev/tap-id".to_owned(), id.to_owned()),
    ]));
    trigger.data = json!({"spec": {}});
    trigger
}

fn managed_secret(namespace: &str, id: &str) -> DynamicObject {
    let mut secret = object(
        &resource("", "v1", "Secret", "secrets"),
        namespace,
        &format!("{id}-token"),
    );
    secret.metadata.labels = Some(BTreeMap::from([
        ("knect.dev/managed-by".to_owned(), "knect".to_owned()),
        ("knect.dev/tap-id".to_owned(), id.to_owned()),
    ]));
    secret.data = json!({"data": {"token": "cmVkYWN0ZWQ="}});
    secret
}

fn manager(
    reader: &FakeReader,
    max_taps: u32,
    max_events: u32,
    ttl_secs: u64,
) -> TriggerTapManager {
    let config = TapManagerConfig {
        enabled: true,
        callback_bind: "127.0.0.1:7421".to_owned(),
        callback_host: String::new(),
        max_taps,
        max_events,
        ttl_secs,
    };
    let manager = TriggerTapManager::new(Arc::new(reader.clone()), config);
    manager.set_callback_addr("127.0.0.1:7421".parse().expect("socket address"));
    manager
}

fn params(target: TapTarget, max_events: Option<u32>, ttl_secs: Option<u64>) -> TapStartParams {
    TapStartParams {
        target,
        filter: None,
        max_events,
        ttl_secs,
    }
}

fn broker_target() -> TapTarget {
    TapTarget {
        kind: TapTargetKind::Broker,
        namespace: "default".to_owned(),
        name: "orders".to_owned(),
    }
}

fn token(handle: &TapHandle) -> String {
    handle
        .callback_url
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn binary_request(event_id: &str, event_type: &str) -> TapCallbackRequest {
    let mut headers = BTreeMap::new();
    headers.insert("ce-specversion".to_owned(), "1.0".to_owned());
    headers.insert("ce-id".to_owned(), event_id.to_owned());
    headers.insert("ce-source".to_owned(), "/orders".to_owned());
    headers.insert("ce-type".to_owned(), event_type.to_owned());
    headers.insert("ce-subject".to_owned(), "orders/42".to_owned());
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    TapCallbackRequest {
        headers,
        body: serde_json::to_vec(&fixture("order_created_data.json")).expect("fixture JSON"),
        received_at: Utc::now(),
    }
}

fn find_created(reader: &FakeReader, kind: &str) -> DynamicObject {
    reader
        .create_calls()
        .into_iter()
        .find(|object| {
            object
                .types
                .as_ref()
                .is_some_and(|types| types.kind == kind)
        })
        .expect("created object of requested kind")
}

fn target_trigger(namespace: &str, name: &str, broker: &str) -> DynamicObject {
    let mut trigger = object(
        &resource("eventing.knative.dev", "v1", "Trigger", "triggers"),
        namespace,
        name,
    );
    trigger.data = json!({
        "spec": {
            "brokerRef": {
                "apiVersion": "eventing.knative.dev/v1",
                "kind": "Broker",
                "name": broker,
            },
            "filter": {
                "attributes": {"type": "order.created"}
            }
        }
    });
    trigger
}

#[tokio::test]
async fn tap_manager_targets_a_trigger_through_its_broker_filter() {
    let reader = FakeReader::new()
        .with_object(broker("default", "orders"))
        .with_object(target_trigger("default", "order-created", "orders"));
    let manager = manager(&reader, 2, 10, 60);
    let handle = manager
        .start(params(
            TapTarget {
                kind: TapTargetKind::Trigger,
                namespace: "default".to_owned(),
                name: "order-created".to_owned(),
            },
            None,
            None,
        ))
        .await
        .expect("trigger target tap starts");
    let trigger = find_created(&reader, "Trigger");
    assert_eq!(trigger.metadata.name.as_deref(), Some(handle.id.as_str()));
    assert_eq!(
        trigger
            .data
            .pointer("/spec/brokerRef/name")
            .and_then(Value::as_str),
        Some("orders")
    );
    assert_eq!(
        trigger
            .data
            .pointer("/spec/filter/attributes/type")
            .and_then(Value::as_str),
        Some("order.created")
    );
}

#[tokio::test]
async fn tap_manager_rejects_a_missing_target() {
    let reader = FakeReader::new();
    let manager = manager(&reader, 2, 10, 60);
    let error = manager
        .start(params(broker_target(), None, None))
        .await
        .expect_err("missing broker is rejected");
    assert!(error.to_string().contains("kubernetes error"));
    assert_eq!(manager.active_count(), 0);
    assert!(manager.list().await.expect("list").is_empty());
}

#[tokio::test]
async fn tap_manager_start_creates_trigger_secret_and_callback_url() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 2, 60);
    let handle = manager
        .start(params(broker_target(), None, None))
        .await
        .expect("tap starts");

    assert_eq!(handle.state, TapState::Active);
    assert_eq!(handle.target, broker_target());
    assert_eq!(handle.max_events, 2);
    assert!(
        handle
            .callback_url
            .starts_with("http://127.0.0.1:7421/tap/tap-")
    );
    assert!(
        handle
            .callback_url
            .ends_with(&format!("/{}", token(&handle)))
    );

    let trigger = find_created(&reader, "Trigger");
    assert_eq!(trigger.metadata.name.as_deref(), Some(handle.id.as_str()));
    assert_eq!(
        trigger
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("knect.dev/managed-by")),
        Some(&"knect".to_owned())
    );
    assert_eq!(
        trigger
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("knect.dev/tap-id")),
        Some(&handle.id)
    );
    assert_eq!(
        trigger
            .data
            .pointer("/spec/brokerRef/name")
            .and_then(Value::as_str),
        Some("orders")
    );
    assert_eq!(
        trigger
            .data
            .pointer("/spec/subscriber/uri")
            .and_then(Value::as_str),
        Some(handle.callback_url.as_str())
    );

    let secret = find_created(&reader, "Secret");
    assert_eq!(
        secret.metadata.name.as_deref(),
        Some(format!("{}-token", handle.id).as_str())
    );
    assert!(
        secret
            .data
            .pointer("/data/token")
            .and_then(Value::as_str)
            .is_some_and(|token| !token.is_empty())
    );

    assert!(manager.list().await.expect("list").len() == 1);
    let stopped = manager.stop(handle.id.clone()).await.expect("stop");
    assert!(stopped.removed);
    let again = manager
        .stop(handle.id.clone())
        .await
        .expect("idempotent stop");
    assert!(!again.removed);
    assert!(manager.list().await.expect("list").is_empty());
    let deleted: Vec<(String, String)> = reader
        .delete_calls()
        .into_iter()
        .map(|call| (call.name, call.gvk.kind))
        .collect();
    assert!(deleted.contains(&(handle.id.clone(), "Trigger".to_owned())));
    assert!(deleted.contains(&(format!("{}-token", handle.id), "Secret".to_owned())));
    assert_eq!(manager.active_count(), 0);
}

#[tokio::test]
async fn tap_manager_delivers_binary_events_into_a_bounded_ring() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 2, 60);
    let handle = manager
        .start(params(broker_target(), Some(2), None))
        .await
        .expect("tap starts");
    let token = token(&handle);

    let mut first = binary_request("ce-1", "order.created");
    first
        .headers
        .insert("authorization".to_owned(), "Bearer super-secret".to_owned());
    let delivery = manager
        .deliver(&handle.id, &token, first)
        .await
        .expect("delivery");
    match delivery {
        TapDelivery::Published(event) => {
            assert_eq!(event.ce_id, "ce-1");
            assert_eq!(event.ce_source, "/orders");
            assert_eq!(event.ce_type, "order.created");
            assert_eq!(event.ce_subject.as_deref(), Some("orders/42"));
            assert_eq!(event.payload["order_id"], "42");
            assert_eq!(
                event.headers.get("authorization").map(String::as_str),
                Some("[REDACTED]")
            );
            assert_eq!(
                event.headers.get("content-type").map(String::as_str),
                Some("application/json")
            );
        }
        TapDelivery::Filtered => panic!("event must match"),
    }

    manager
        .deliver(&handle.id, &token, binary_request("ce-2", "order.created"))
        .await
        .expect("second delivery");
    manager
        .deliver(&handle.id, &token, binary_request("ce-3", "order.created"))
        .await
        .expect("third delivery");

    let events = manager.events(&handle.id);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].ce_id, "ce-2");
    assert_eq!(events[1].ce_id, "ce-3");
}

#[tokio::test]
async fn tap_manager_rejects_bad_tokens_filters_and_malformed_events() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 10, 60);
    let mut start = params(broker_target(), None, None);
    start.filter = Some(TapFilter {
        attributes: BTreeMap::from([("type".to_owned(), "order.created".to_owned())]),
    });
    let handle = manager.start(start).await.expect("tap starts");
    let token = token(&handle);

    let unauthorized = manager
        .deliver(
            &handle.id,
            "wrong-token",
            binary_request("ce-1", "order.created"),
        )
        .await
        .expect_err("wrong token is rejected");
    assert!(unauthorized.to_string().contains("unauthorized"));

    let filtered = manager
        .deliver(&handle.id, &token, binary_request("ce-2", "order.deleted"))
        .await
        .expect("authenticated delivery");
    assert_eq!(filtered, TapDelivery::Filtered);

    let mut malformed = binary_request("ce-3", "order.created");
    malformed.headers.remove("ce-id");
    let error = manager
        .deliver(&handle.id, &token, malformed)
        .await
        .expect_err("missing ce-id is rejected");
    assert!(error.to_string().contains("invalid CloudEvent"));

    let mut oversized = binary_request("ce-4", "order.created");
    oversized.body = vec![b'x'; 1_048_577];
    let error = manager
        .deliver(&handle.id, &token, oversized)
        .await
        .expect_err("oversized body is rejected");
    assert!(error.to_string().contains("exceeds"));

    assert!(manager.events(&handle.id).is_empty());
}

#[tokio::test]
async fn tap_manager_parses_structured_cloud_events() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 10, 60);
    let handle = manager
        .start(params(broker_target(), None, None))
        .await
        .expect("tap starts");
    let token = token(&handle);
    let body = serde_json::to_vec(&fixture("order_created.json")).expect("fixture JSON");
    let request = TapCallbackRequest {
        headers: BTreeMap::from([(
            "content-type".to_owned(),
            "application/cloudevents+json".to_owned(),
        )]),
        body,
        received_at: Utc::now(),
    };
    match manager
        .deliver(&handle.id, &token, request)
        .await
        .expect("structured delivery")
    {
        TapDelivery::Published(event) => {
            assert_eq!(event.ce_id, "ce-order-1");
            assert_eq!(event.ce_type, "order.created");
            assert_eq!(event.payload["total"], 19.99);
        }
        TapDelivery::Filtered => panic!("structured event must match"),
    }
}

#[tokio::test]
async fn tap_manager_reaps_expired_taps_and_retains_recent_history() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 10, 60);
    let handle = manager
        .start(params(broker_target(), None, Some(1)))
        .await
        .expect("tap starts");

    let expired = manager
        .reap_at(Utc::now() + ChronoDuration::seconds(2))
        .await
        .expect("reap");
    assert_eq!(expired, vec![handle.id.clone()]);
    let listed = manager.list().await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, TapState::Expired);
    assert!(manager.events(&handle.id).is_empty());
    assert!(
        reader
            .delete_calls()
            .iter()
            .any(|call| call.name == handle.id)
    );
    assert!(
        manager
            .reap_at(Utc::now() + ChronoDuration::seconds(3))
            .await
            .expect("second reap")
            .is_empty()
    );
}

#[tokio::test]
async fn tap_manager_ttl_reaper_expires_live_taps() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 10, 1);
    let handle = manager
        .start(params(broker_target(), None, Some(1)))
        .await
        .expect("tap starts");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    while manager.active_count() > 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(manager.active_count(), 0, "TTL reaper must expire the tap");
    let listed = manager.list().await.expect("list");
    assert_eq!(listed[0].id, handle.id);
    assert_eq!(listed[0].state, TapState::Expired);
}

#[tokio::test]
async fn tap_manager_startup_gc_deletes_orphaned_managed_objects() {
    let reader = FakeReader::new()
        .with_object(broker("default", "orders"))
        .with_object(managed_trigger("default", "tap-orphan"))
        .with_object(managed_secret("default", "tap-orphan"));
    let manager = manager(&reader, 2, 10, 60);

    let removed = manager.gc_startup().await.expect("gc");
    assert_eq!(removed, vec!["tap-orphan".to_owned()]);
    let deleted: Vec<String> = reader
        .delete_calls()
        .into_iter()
        .map(|call| call.name)
        .collect();
    assert!(deleted.contains(&"tap-orphan".to_owned()));
    assert!(deleted.contains(&"tap-orphan-token".to_owned()));
    assert_eq!(manager.active_count(), 0);
}

#[tokio::test]
async fn tap_manager_enforces_limits_and_shutdown_removes_every_tap() {
    let reader = FakeReader::new().with_object(broker("default", "orders"));
    let manager = manager(&reader, 2, 10, 60);
    let first = manager
        .start(params(broker_target(), None, None))
        .await
        .expect("first tap");
    let second = manager
        .start(params(broker_target(), None, None))
        .await
        .expect("second tap");
    let error = manager
        .start(params(broker_target(), None, None))
        .await
        .expect_err("third tap exceeds the limit");
    assert!(error.to_string().contains("limit reached"));

    manager.shutdown().await.expect("shutdown");
    assert_eq!(manager.active_count(), 0);
    assert!(manager.list().await.expect("list").is_empty());
    let deleted: Vec<String> = reader
        .delete_calls()
        .into_iter()
        .map(|call| call.name)
        .collect();
    for tap in [&first, &second] {
        assert!(deleted.contains(&tap.id));
        assert!(deleted.contains(&format!("{}-token", tap.id)));
    }
    manager.shutdown().await.expect("idempotent shutdown");
}
