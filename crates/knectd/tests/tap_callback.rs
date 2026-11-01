#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Live tap RPC lifecycle over the real token-guarded callback listener.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use knect_core::config::{Config, TapConfig};
use knect_core::kube::fake::FakeReader;
use knect_core::tap::{TapManagerConfig, TriggerTapManager};
use knect_proto::events::{SseEvent, SseEventKind};
use knect_proto::tap::TapHandle;
use knect_proto::{RpcRequest, RpcResponse};
use knectd::transport::sse::format_frame;
use knectd::{AppState, RpcDispatcher};
use kube::api::{ApiResource, DynamicObject};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures/tap")
        .join(name);
    std::fs::read(path).expect("tap fixture is readable")
}

fn broker() -> DynamicObject {
    let resource = ApiResource {
        group: "eventing.knative.dev".to_owned(),
        version: "v1".to_owned(),
        api_version: "eventing.knative.dev/v1".to_owned(),
        kind: "Broker".to_owned(),
        plural: "brokers".to_owned(),
    };
    let mut broker = DynamicObject::new("orders", &resource);
    broker.metadata.namespace = Some("default".to_owned());
    broker.data = json!({"spec": {}});
    broker
}

fn state(reader: &FakeReader) -> AppState {
    let config = Config {
        tap: TapConfig {
            enabled: true,
            callback_bind: "127.0.0.1:0".to_owned(),
            callback_host: String::new(),
            max_taps: 2,
            max_events: 10,
            ttl_secs: 60,
        },
        ..Config::default()
    };
    let reader: Arc<dyn knect_core::ClusterReader> = Arc::new(reader.clone());
    let manager = Arc::new(TriggerTapManager::new(
        Arc::clone(&reader),
        TapManagerConfig::from(&config.tap),
    ));
    AppState::new(config, false)
        .with_cluster_reader(reader)
        .with_tap(manager)
}

async fn rpc(state: &AppState, id: i64, method: &str, params: Value) -> Value {
    let request = RpcRequest::new(id, method, Some(params));
    let response: RpcResponse = RpcDispatcher::new(state.clone())
        .dispatch_async(request)
        .await
        .expect("RPC response");
    if let Some(error) = response.error {
        panic!("RPC {method} failed: {error:?}");
    }
    response.result.expect("RPC result")
}

async fn post(url: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, String) {
    let rest = url.strip_prefix("http://").expect("http callback URL");
    let (authority, path) = rest.split_once('/').expect("callback path");
    let mut stream = TcpStream::connect(authority)
        .await
        .expect("connect callback");
    let mut request = format!(
        "POST /{path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write headers");
    stream.write_all(body).await.expect("write body");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse::<u16>().ok())
        .expect("HTTP status");
    (status, text)
}

async fn tap_event(receiver: &mut broadcast::Receiver<SseEvent>) -> SseEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = receiver.recv().await
                && matches!(event.kind, SseEventKind::TapEvent { .. })
            {
                return event;
            }
        }
    })
    .await
    .expect("tap event before timeout")
}

#[tokio::test]
async fn tap_rpc_lifecycle_delivers_a_scripted_cloud_event() {
    let reader = FakeReader::new().with_object(broker());
    let state = state(&reader);
    let mut events = state.events.subscribe();

    let result = rpc(
        &state,
        1,
        "tap.start",
        json!({
            "target": {"kind": "broker", "namespace": "default", "name": "orders"},
            "max_events": 5
        }),
    )
    .await;
    let handle: TapHandle = serde_json::from_value(result).expect("tap handle");
    assert!(handle.callback_url.contains(&handle.id));
    assert_eq!(state.tap.active_count(), 1);

    let created = reader.create_calls();
    assert!(created.iter().any(|object| {
        object
            .types
            .as_ref()
            .is_some_and(|types| types.kind == "Trigger")
    }));

    let mut wrong = handle.callback_url.clone();
    let (prefix, _) = wrong.rsplit_once('/').expect("token segment");
    wrong = format!("{prefix}/wrong-token");
    let (status, _) = post(
        &wrong,
        &[
            ("Ce-Specversion", "1.0"),
            ("Ce-Id", "wrong"),
            ("Ce-Source", "/orders"),
            ("Ce-Type", "order.created"),
            ("Content-Type", "application/json"),
        ],
        &fixture("order_created_data.json"),
    )
    .await;
    assert_eq!(status, 401, "wrong token must be rejected");

    let (status, _) = post(
        &handle.callback_url,
        &[
            ("Ce-Specversion", "1.0"),
            ("Ce-Id", "ce-order-1"),
            ("Ce-Source", "/orders"),
            ("Ce-Type", "order.created"),
            ("Ce-Subject", "orders/42"),
            ("Authorization", "Bearer must-not-leak"),
            ("Content-Type", "application/json"),
        ],
        &fixture("order_created_data.json"),
    )
    .await;
    assert_eq!(status, 204, "authenticated callback is acknowledged");

    let event = tap_event(&mut events).await;
    let frame = format_frame(&event);
    assert!(frame.contains("event: tap.event"));
    match event.kind {
        SseEventKind::TapEvent { data } => {
            assert_eq!(data.ce_id, "ce-order-1");
            assert_eq!(data.ce_type, "order.created");
            assert_eq!(data.payload["order_id"], "42");
            assert_eq!(
                data.headers.get("authorization").map(String::as_str),
                Some("[REDACTED]")
            );
            let encoded = serde_json::to_value(&data).expect("event JSON");
            assert!(!encoded.to_string().contains("must-not-leak"));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let listed: Vec<TapHandle> =
        serde_json::from_value(rpc(&state, 2, "tap.list", json!({})).await).expect("tap list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, handle.id);

    let stopped: Value = rpc(&state, 3, "tap.stop", json!({"tap_id": handle.id})).await;
    assert!(stopped["removed"].as_bool().unwrap_or(false));
    let again: Value = rpc(&state, 4, "tap.stop", json!({"tap_id": handle.id})).await;
    assert!(!again["removed"].as_bool().unwrap_or(true));
    assert_eq!(state.tap.active_count(), 0);
    let listed: Vec<TapHandle> =
        serde_json::from_value(rpc(&state, 5, "tap.list", json!({})).await).expect("tap list");
    assert!(listed.is_empty());
    let deleted: Vec<String> = reader
        .delete_calls()
        .into_iter()
        .map(|call| call.name)
        .collect();
    assert!(deleted.contains(&handle.id));
}

#[tokio::test]
async fn tap_rpc_rejects_fixture_mode_and_invalid_targets() {
    let reader = FakeReader::new();
    let fixture_state = AppState::new(Config::default(), true);
    let request = RpcRequest::new(
        1,
        "tap.start",
        Some(json!({
            "target": {"kind": "broker", "namespace": "default", "name": "orders"}
        })),
    );
    let response = RpcDispatcher::new(fixture_state)
        .dispatch_async(request)
        .await
        .expect("response");
    assert_eq!(
        response.error.expect("fixture start rejects").code,
        knect_proto::error::TAP_FORBIDDEN
    );

    let state = state(&reader);
    let request = RpcRequest::new(
        2,
        "tap.start",
        Some(json!({
            "target": {"kind": "unknown", "namespace": "default", "name": "orders"}
        })),
    );
    let response = RpcDispatcher::new(state.clone())
        .dispatch_async(request)
        .await
        .expect("response");
    assert_eq!(
        response.error.expect("invalid kind rejects").code,
        knect_proto::error::INVALID_PARAMS
    );
}
