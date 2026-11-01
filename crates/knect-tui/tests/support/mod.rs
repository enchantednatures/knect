//! Test-only fixtures and an HTTP stub daemon implementing the frozen RPC
//! contract. Never linked into the production binary.

#![allow(dead_code)]

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use chrono::Utc;
use knect_proto::error::{METHOD_NOT_FOUND, NOT_FOUND};
use knect_proto::events::{Heartbeat, SseEvent, SseEventKind};
use knect_proto::graph::Topology;
use knect_proto::ids::Gvk;
use knect_proto::rpc::{
    DaemonStatus, GetNodeDetailsParams, LogsSubscribeParams, WatcherStatus, methods,
};
use knect_proto::tap::{LogLine, LogStream};
use knect_proto::{NodeDetails, ResourceId, RpcError, RpcRequest, RpcResponse};
use knect_tui::app::state::{AppState, ConnectionStatus, InspectorTab};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub const REVISION_NAME: &str = "hello-00001";

pub fn repository_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

pub fn fixture_topology() -> Topology {
    let path = repository_path("testdata/contracts/topology.sample.json");
    let raw = std::fs::read_to_string(&path).expect("read topology fixture");
    serde_json::from_str(&raw).expect("decode topology fixture")
}

pub fn fixture_details() -> NodeDetails {
    let path = repository_path("testdata/contracts/node_details.sample.json");
    let raw = std::fs::read_to_string(&path).expect("read details fixture");
    serde_json::from_str(&raw).expect("decode details fixture")
}

pub fn revision_id() -> ResourceId {
    ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Revision",
        "default",
        REVISION_NAME,
    )
}

pub fn fixture_status() -> DaemonStatus {
    DaemonStatus {
        version: "0.1.0".to_owned(),
        schema_version: knect_proto::SCHEMA_VERSION,
        context: "kind-knect-e2e".to_owned(),
        generation: 42,
        watchers: vec![WatcherStatus {
            gvk: Gvk::new("serving.knative.dev", "v1", "Service"),
            resource_count: 4,
            healthy: true,
            last_event: None,
        }],
        uptime_secs: 120,
        actions_enabled: false,
        tap_count: 0,
        started_at: None,
        cache_enabled: true,
        last_resync: None,
    }
}

pub fn snapshot_state() -> AppState {
    let mut state = AppState::new();
    state.set_endpoint("unix:/tmp/knectd.sock");
    state.daemon = Some(fixture_status());
    state.set_connection(ConnectionStatus::Connected);
    state.set_topology(fixture_topology());
    assert!(state.select_resource(&revision_id()));
    state.details = Some(fixture_details());
    state.inspector_tab = InspectorTab::Mutations;
    state.dirty = false;
    state
}

pub struct StubServer {
    pub url: String,
    handle: JoinHandle<()>,
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub async fn start_stub_daemon() -> StubServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub listener");
    let address = listener.local_addr().expect("stub address");
    let state = Arc::new(StubState {
        topology: fixture_topology(),
        details: fixture_details(),
    });
    let app = Router::new()
        .route("/rpc", post(rpc))
        .route("/healthz", get(health))
        .route("/events", get(events))
        .with_state(state);
    let handle = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            eprintln!("stub daemon stopped: {error}");
        }
    });
    let url = format!("http://{address}");
    for _ in 0..100 {
        let ready = reqwest::get(format!("{url}/healthz"))
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false);
        if ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    StubServer { url, handle }
}

struct StubState {
    topology: Topology,
    details: NodeDetails,
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn rpc(
    State(state): State<Arc<StubState>>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let id = request.id.clone();
    let outcome = dispatch(&state, &request);
    Json(match outcome {
        Ok(result) => RpcResponse::success(id, result),
        Err((code, message)) => RpcResponse::failure(id, RpcError::new(code, message)),
    })
}

fn dispatch(state: &StubState, request: &RpcRequest) -> Result<Value, (i32, String)> {
    match request.method.as_str() {
        methods::DAEMON_STATUS => encode(fixture_status()),
        methods::GRAPH_GET_TOPOLOGY => encode(&state.topology),
        methods::GRAPH_GET_NODE_DETAILS => {
            let params: GetNodeDetailsParams =
                decode_params(request)?.ok_or((NOT_FOUND, "missing params".to_owned()))?;
            details_for(state, &params.id)
        }
        methods::GRAPH_REFRESH => Ok(json!({"generation": state.topology.generation})),
        methods::LOGS_SUBSCRIBE => {
            let _: Option<LogsSubscribeParams> = decode_params(request)?;
            Ok(json!({"subscription_id": "stub-logs"}))
        }
        methods::TAP_LIST => Ok(json!([])),
        methods::EVENTS_SUBSCRIBE => Err((
            METHOD_NOT_FOUND,
            "use GET /events on the HTTP transport".to_owned(),
        )),
        other => Err((METHOD_NOT_FOUND, format!("unhandled method {other}"))),
    }
}

fn details_for(state: &StubState, id: &ResourceId) -> Result<Value, (i32, String)> {
    if &state.details.id == id {
        return encode(&state.details);
    }
    let Some(node) = state.topology.nodes.iter().find(|node| &node.id == id) else {
        return Err((NOT_FOUND, format!("resource {id} not found")));
    };
    let details = NodeDetails {
        id: node.id.clone(),
        category: node.category,
        name: node.name.clone(),
        namespace: node.namespace.clone(),
        health: node.health,
        summary: node.summary.clone(),
        yaml: Some(format!(
            "apiVersion: {}\nkind: {}\nmetadata:\n  name: {}\n",
            node.id.api_version, node.id.kind, node.id.name
        )),
        yaml_redacted: true,
        mutations: Vec::new(),
        policies: Vec::new(),
        links: Vec::new(),
        storage: Vec::new(),
        warnings: Vec::new(),
        metrics: None,
        logs: Vec::new(),
    };
    encode(&details)
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value, (i32, String)> {
    serde_json::to_value(value)
        .map_err(|error| (knect_proto::error::INTERNAL_ERROR, error.to_string()))
}

fn decode_params<T: serde::de::DeserializeOwned>(
    request: &RpcRequest,
) -> Result<Option<T>, (i32, String)> {
    match request.params.as_ref() {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| (knect_proto::error::INVALID_PARAMS, error.to_string())),
    }
}

async fn events(
    State(state): State<Arc<StubState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let snapshot = SseEvent::new(
        1,
        Utc::now(),
        SseEventKind::Snapshot {
            data: state.topology.clone(),
        },
    );
    let log = SseEvent::new(
        2,
        Utc::now(),
        SseEventKind::LogLine {
            data: LogLine {
                namespace: "default".to_owned(),
                pod: "hello-00001-deployment-abc".to_owned(),
                container: "user-container".to_owned(),
                timestamp: Utc::now(),
                message: "listening on :8080".to_owned(),
                stream: LogStream::Stdout,
            },
        },
    );
    let stream = async_stream::stream! {
        yield Ok(frame(&snapshot));
        tokio::time::sleep(Duration::from_millis(150)).await;
        yield Ok(frame(&log));
        let mut sequence = 3_u64;
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let heartbeat = SseEvent::new(
                sequence,
                Utc::now(),
                SseEventKind::Heartbeat {
                    data: Heartbeat {
                        uptime_secs: sequence,
                        generation: 42,
                        node_count: 4,
                        edge_count: 3,
                        ts: Utc::now(),
                    },
                },
            );
            sequence = sequence.saturating_add(1);
            yield Ok(frame(&heartbeat));
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn frame(event: &SseEvent) -> Event {
    Event::default()
        .id(event.seq.to_string())
        .event(event.event_type())
        .data(serde_json::to_string(event).expect("encode SSE event"))
}
