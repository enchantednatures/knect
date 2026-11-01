//! Newline-delimited JSON-RPC framing for the Unix domain socket.
//!
//! One request object per line, one response object per line. Notifications
//! produce no line. `events.subscribe` first returns its normal JSON-RPC result
//! and then streams `SseEvent` JSON objects on the same connection.

use std::sync::atomic::{AtomicU64, Ordering};

use knect_proto::error::{INVALID_REQUEST, PARSE_ERROR};
use knect_proto::events::{SseEvent, SseEventKind};
use knect_proto::rpc::{EventsSubscribeParams, EventsSubscribeResult, methods};
use knect_proto::{RpcId, RpcRequest, RpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::AppState;
use crate::error::DaemonError;
use crate::rpc::RpcDispatcher;

static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);
const RESPONSE_QUEUE: usize = 256;

/// Serves JSONL requests until the peer closes the connection.
pub async fn serve_connection<S>(state: AppState, stream: S) -> Result<(), DaemonError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    serve_io(state, reader, writer).await
}

/// Generic JSONL loop used by the UDS listener and unit tests.
pub async fn serve_io<R, W>(state: AppState, reader: R, writer: W) -> Result<(), DaemonError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    let dispatcher = RpcDispatcher::new(state.clone());
    let (writer, writer_task) = spawn_writer(writer);
    let mut subscriptions: Vec<JoinHandle<()>> = Vec::new();
    let mut failure: Option<DaemonError> = None;

    'read: loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                failure = Some(DaemonError::Transport(format!(
                    "JSONL read failed: {error}"
                )));
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<RpcRequest>(trimmed) {
            Ok(request) => request,
            Err(error) => {
                let response = RpcResponse::failure(
                    RpcId::Null,
                    knect_proto::RpcError::new(
                        if trimmed.starts_with('[') {
                            INVALID_REQUEST
                        } else {
                            PARSE_ERROR
                        },
                        format!("invalid JSON-RPC line: {error}"),
                    ),
                );
                if let Err(error) = send_response(&writer, &response).await {
                    failure = Some(error);
                    break;
                }
                continue;
            }
        };

        if request.method == methods::EVENTS_SUBSCRIBE && !request.id.is_null() {
            let params = match request.params.as_ref() {
                None | Some(serde_json::Value::Null) => EventsSubscribeParams::default(),
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(params) => params,
                    Err(error) => {
                        let response = RpcResponse::failure(
                            request.id,
                            knect_proto::RpcError::new(
                                knect_proto::error::INVALID_PARAMS,
                                format!("invalid params: {error}"),
                            ),
                        );
                        if let Err(error) = send_response(&writer, &response).await {
                            failure = Some(error);
                            break 'read;
                        }
                        continue;
                    }
                },
            };
            let subscription_id = format!(
                "sub-{}-{}",
                std::process::id(),
                SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let response = RpcResponse::success(
                request.id,
                serde_json::json!(EventsSubscribeResult { subscription_id }),
            );
            if let Err(error) = send_response(&writer, &response).await {
                failure = Some(error);
                break;
            }
            subscriptions.push(spawn_subscription(state.clone(), params, writer.clone()));
            continue;
        }

        if let Some(response) = dispatcher.dispatch_async(request).await
            && let Err(error) = send_response(&writer, &response).await
        {
            failure = Some(error);
            break;
        }
    }

    for subscription in subscriptions {
        subscription.abort();
        let _ = subscription.await;
    }
    drop(writer);
    let _ = writer_task.await;
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn spawn_writer<W>(writer: W) -> (mpsc::Sender<String>, JoinHandle<()>)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (sender, mut receiver) = mpsc::channel::<String>(RESPONSE_QUEUE);
    let task = tokio::spawn(async move {
        let mut writer = BufWriter::new(writer);
        while let Some(line) = receiver.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if writer.write_all(b"\n").await.is_err() {
                break;
            }
            if writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.flush().await;
    });
    (sender, task)
}

async fn send_response(
    writer: &mpsc::Sender<String>,
    response: &RpcResponse,
) -> Result<(), DaemonError> {
    let encoded = serde_json::to_string(response)
        .map_err(|error| DaemonError::Rpc(format!("cannot encode response: {error}")))?;
    writer
        .send(encoded)
        .await
        .map_err(|_| DaemonError::Transport("JSONL writer closed".to_owned()))
}

fn spawn_subscription(
    state: AppState,
    params: EventsSubscribeParams,
    writer: mpsc::Sender<String>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut receiver = state.events.subscribe();
        let snapshot = state.events.next_event(SseEventKind::Snapshot {
            data: (*state.topology()).clone(),
        });
        let mut last_sent = snapshot.seq;
        if send_event(&writer, &snapshot, &params).await.is_err() {
            return;
        }
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.seq <= last_sent {
                        continue;
                    }
                    last_sent = event.seq;
                    if send_event(&writer, &event, &params).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let snapshot = state.events.next_event(SseEventKind::Snapshot {
                        data: (*state.topology()).clone(),
                    });
                    last_sent = snapshot.seq;
                    if send_event(&writer, &snapshot, &params).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

async fn send_event(
    writer: &mpsc::Sender<String>,
    event: &SseEvent,
    params: &EventsSubscribeParams,
) -> Result<(), ()> {
    if !topic_allowed(params, event) {
        return Ok(());
    }
    let encoded = serde_json::to_string(event).map_err(|error| {
        tracing::warn!(%error, "cannot encode JSONL subscription event");
    })?;
    writer.send(encoded).await.map_err(|_| ())
}

fn topic_allowed(params: &EventsSubscribeParams, event: &SseEvent) -> bool {
    params
        .topics
        .as_ref()
        .is_none_or(|topics| topics.iter().any(|topic| topic == event.event_type()))
}

/// Decodes and dispatches one JSONL line, returning the response line.
#[must_use]
pub async fn handle_line(dispatcher: &RpcDispatcher, line: &str) -> Option<RpcResponse> {
    match serde_json::from_str::<RpcRequest>(line) {
        Ok(request) => dispatcher.dispatch_async(request).await,
        Err(error) => Some(RpcResponse::failure(
            RpcId::Null,
            knect_proto::RpcError::new(
                if line.starts_with('[') {
                    INVALID_REQUEST
                } else {
                    PARSE_ERROR
                },
                format!("invalid JSON-RPC line: {error}"),
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_core::config::Config;
    use knect_proto::graph::TopologyStats;
    use tokio::io::duplex;

    fn state() -> AppState {
        AppState::new(Config::default(), true)
    }

    #[tokio::test]
    async fn handle_line_dispatches_and_reports_parse_errors() {
        let state = state();
        let dispatcher = RpcDispatcher::new(state);
        let response = handle_line(
            &dispatcher,
            r#"{"jsonrpc":"2.0","id":1,"method":"daemon.status","params":{}}"#,
        )
        .await
        .expect("response");
        assert!(response.error.is_none());
        let error = handle_line(&dispatcher, "{").await.expect("error response");
        assert_eq!(
            error.error.map(|error| error.code),
            Some(knect_proto::error::PARSE_ERROR)
        );
    }

    #[tokio::test]
    async fn serve_io_round_trips_one_request() {
        let state = state();
        let (client, server) = duplex(4096);
        let handle = tokio::spawn(async move {
            let (reader, writer) = tokio::io::split(server);
            serve_io(state, reader, writer).await
        });
        let (reader, mut writer) = tokio::io::split(client);
        writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"daemon.status\",\"params\":{}}\n",
            )
            .await
            .expect("write");
        writer.flush().await.expect("flush");
        let mut lines = BufReader::new(reader).lines();
        let line = lines.next_line().await.expect("read").expect("line");
        let response: RpcResponse = serde_json::from_str(&line).expect("response JSON");
        assert_eq!(response.id, RpcId::Number(7));
        assert!(response.error.is_none());
        drop(lines);
        drop(writer);
        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn events_subscribe_streams_a_snapshot_then_matching_graph_patches() {
        let state = state();
        let server_state = state.clone();
        let (client, server) = duplex(16 * 1024);
        let handle = tokio::spawn(async move {
            let (reader, writer) = tokio::io::split(server);
            serve_io(server_state, reader, writer).await
        });
        let (reader, mut writer) = tokio::io::split(client);
        writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"events.subscribe\",\"params\":{}}\n",
            )
            .await
            .expect("write");
        writer.flush().await.expect("flush");
        let mut lines = BufReader::new(reader).lines();
        let result: RpcResponse =
            serde_json::from_str(&lines.next_line().await.expect("read").expect("line"))
                .expect("result JSON");
        assert_eq!(result.id, RpcId::Number(9));
        assert!(result.result.expect("subscription result")["subscription_id"].is_string());

        let snapshot: SseEvent =
            serde_json::from_str(&lines.next_line().await.expect("read").expect("line"))
                .expect("snapshot JSON");
        assert_eq!(snapshot.event_type(), "snapshot");

        let mut topology = crate::app::empty_topology("fixture", 1);
        topology.stats = TopologyStats {
            node_count: 1,
            ..TopologyStats::default()
        };
        let event = state.set_topology_and_publish(topology);
        let patch: SseEvent =
            serde_json::from_str(&lines.next_line().await.expect("read").expect("line"))
                .expect("patch JSON");
        assert_eq!(patch.event_type(), "graph.patch");
        assert_eq!(patch.seq, event.seq);
        match patch.kind {
            SseEventKind::GraphPatch { data } => assert_eq!(data.seq, patch.seq),
            other => panic!("unexpected event: {other:?}"),
        }

        drop(lines);
        drop(writer);
        handle.abort();
        let _ = handle.await;
    }
}
