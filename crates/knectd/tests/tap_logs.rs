#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `logs.subscribe` streams protocol `LogLine` events with redaction.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use knect_core::ClusterReader;
use knect_core::config::Config;
use knect_core::kube::fake::FakeReader;
use knect_proto::events::{SseEvent, SseEventKind};
use knect_proto::tap::{LogLine, LogStream};
use knect_proto::{RpcRequest, RpcResponse};
use knectd::{AppState, RpcDispatcher};
use serde_json::{Value, json};
use tokio::sync::broadcast;

fn state(reader: &FakeReader) -> AppState {
    let reader: Arc<dyn ClusterReader> = Arc::new(reader.clone());
    AppState::new(Config::default(), true).with_cluster_reader(reader)
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

async fn log_event(receiver: &mut broadcast::Receiver<SseEvent>) -> LogLine {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = receiver.recv().await
                && let SseEventKind::LogLine { data } = event.kind
            {
                return data;
            }
        }
    })
    .await
    .expect("log event before timeout")
}

#[tokio::test]
async fn tap_logs_subscribe_streams_redacted_log_lines() {
    let reader = FakeReader::new().with_log_stream(vec![
        Ok("2026-11-01T12:00:00Z request handled token=hunter2".to_owned()),
        Ok("2026-11-01T12:00:01Z done".to_owned()),
    ]);
    let state = state(&reader);
    let mut events = state.events.subscribe();

    let result = rpc(
        &state,
        1,
        "logs.subscribe",
        json!({
            "pods": [{"namespace": "default", "name": "pod-1", "container": "user-container"}],
            "tail_lines": 10
        }),
    )
    .await;
    let subscription_id = result["subscription_id"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let first = log_event(&mut events).await;
    assert_eq!(first.namespace, "default");
    assert_eq!(first.pod, "pod-1");
    assert_eq!(first.container, "user-container");
    assert_eq!(first.stream, LogStream::Stdout);
    assert_eq!(
        first.timestamp,
        "2026-11-01T12:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("timestamp")
    );
    assert!(first.message.contains("[REDACTED]"));
    assert!(!first.message.contains("hunter2"));

    let second = log_event(&mut events).await;
    assert_eq!(second.message, "done");

    let calls = reader.log_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].namespace, "default");
    assert_eq!(calls[0].pod, "pod-1");
    assert_eq!(
        calls[0].options.container.as_deref(),
        Some("user-container")
    );
    assert!(calls[0].options.timestamps);
    assert!(calls[0].options.follow);

    let stopped: Value = rpc(
        &state,
        2,
        "logs.stop",
        json!({"subscription_id": subscription_id}),
    )
    .await;
    assert!(stopped["stopped"].as_bool().unwrap_or(false));
    let again: Value = rpc(
        &state,
        3,
        "logs.stop",
        json!({"subscription_id": subscription_id}),
    )
    .await;
    assert!(!again["stopped"].as_bool().unwrap_or(true));
}

#[tokio::test]
async fn tap_logs_filter_drops_non_matching_lines_first() {
    let reader = FakeReader::new().with_log_stream(vec![
        Ok("2026-11-01T12:00:00Z drop this".to_owned()),
        Ok("2026-11-01T12:00:01Z keep this".to_owned()),
    ]);
    let state = state(&reader);
    let mut events = state.events.subscribe();

    let result = rpc(
        &state,
        1,
        "logs.subscribe",
        json!({
            "pods": [{"namespace": "default", "name": "pod-1", "container": "user-container"}],
            "filter": "keep"
        }),
    )
    .await;
    let subscription_id = result["subscription_id"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let only = log_event(&mut events).await;
    assert_eq!(only.message, "keep this");
    let _: Value = rpc(
        &state,
        2,
        "logs.stop",
        json!({"subscription_id": subscription_id}),
    )
    .await;
}
