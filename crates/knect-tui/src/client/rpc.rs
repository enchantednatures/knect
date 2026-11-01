//! Concrete JSON-RPC client for the Unix socket and HTTP transports.

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use knect_proto::rpc::{EventsSubscribeParams, methods};
use knect_proto::{RpcRequest, RpcResponse, SseEvent};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::events;
use super::{ClientError, Endpoint, RpcClient};

/// Largest accepted response or event line before the client refuses it.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Live RPC client used by the TUI and CLI.
#[derive(Debug)]
pub struct RpcClientImpl {
    endpoint: Endpoint,
    timeout: Duration,
    next_id: AtomicI64,
    http: reqwest::Client,
}

impl RpcClientImpl {
    /// Creates a client for one endpoint.
    #[must_use]
    pub fn new(endpoint: Endpoint, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            endpoint,
            timeout,
            next_id: AtomicI64::new(0),
            http,
        }
    }

    /// Endpoint this client talks to.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn next_request(&self, method: &str, params: Option<Value>) -> RpcRequest {
        let id = self
            .next_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        RpcRequest::new(id, method, params)
    }

    async fn unix_call(&self, path: &Path, request: RpcRequest) -> Result<Value, ClientError> {
        let stream = UnixStream::connect(path).await.map_err(|error| {
            ClientError::Transport(format!("cannot connect to {}: {error}", path.display()))
        })?;
        let (reader, mut writer) = stream.into_split();
        let mut line = encode_request(&request)?;
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|error| ClientError::Transport(format!("cannot write request: {error}")))?;
        writer
            .flush()
            .await
            .map_err(|error| ClientError::Transport(format!("cannot flush request: {error}")))?;
        let mut lines = BufReader::new(reader).lines();
        loop {
            let next = lines.next_line().await.map_err(|error| {
                ClientError::Transport(format!("cannot read response: {error}"))
            })?;
            let Some(next) = next else {
                return Err(ClientError::Disconnected(
                    "daemon closed the connection before replying".to_owned(),
                ));
            };
            if next.len() > MAX_LINE_BYTES {
                return Err(ClientError::Protocol(
                    "response line is too large".to_owned(),
                ));
            }
            if next.trim().is_empty() {
                continue;
            }
            let response: RpcResponse = serde_json::from_str(next.trim())
                .map_err(|error| ClientError::Protocol(format!("invalid response: {error}")))?;
            if response.id != request.id {
                continue;
            }
            return response_into_result(response);
        }
    }

    async fn http_call(&self, base: &str, request: RpcRequest) -> Result<Value, ClientError> {
        let url = format!("{base}/rpc");
        let response = self
            .http
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|error| ClientError::Transport(format!("POST {url} failed: {error}")))?;
        let status = response.status();
        let body = response.bytes().await.map_err(|error| {
            ClientError::Transport(format!("cannot read response body: {error}"))
        })?;
        if !status.is_success() {
            return Err(ClientError::HttpStatus {
                status: status.as_u16(),
                body: truncate(&String::from_utf8_lossy(&body), 400),
            });
        }
        let response: RpcResponse = serde_json::from_slice(&body)
            .map_err(|error| ClientError::Protocol(format!("invalid response body: {error}")))?;
        response_into_result(response)
    }
}

impl RpcClient for RpcClientImpl {
    fn call(
        &self,
        method: String,
        params: Option<Value>,
    ) -> BoxFuture<'_, Result<Value, ClientError>> {
        Box::pin(async move {
            let request = self.next_request(&method, params);
            let timeout = self.timeout;
            let work = async {
                match &self.endpoint {
                    Endpoint::Unix(path) => self.unix_call(path, request).await,
                    Endpoint::Http(base) => self.http_call(base, request).await,
                }
            };
            tokio::time::timeout(timeout, work)
                .await
                .map_err(|_| ClientError::Timeout(timeout))?
        })
    }

    fn subscribe(
        &self,
        topics: Option<Vec<String>>,
        last_event_id: Option<u64>,
    ) -> BoxFuture<'_, Result<BoxStream<'static, Result<SseEvent, ClientError>>, ClientError>> {
        let endpoint = self.endpoint.clone();
        let http = self.http.clone();
        let timeout = self.timeout;
        Box::pin(async move {
            match endpoint {
                Endpoint::Unix(path) => unix_subscribe(&path, topics, last_event_id, timeout).await,
                Endpoint::Http(base) => http_subscribe(http, &base, last_event_id).await,
            }
        })
    }
}

fn encode_request(request: &RpcRequest) -> Result<String, ClientError> {
    serde_json::to_string(request)
        .map_err(|error| ClientError::Protocol(format!("cannot encode request: {error}")))
}

fn response_into_result(response: RpcResponse) -> Result<Value, ClientError> {
    if let Some(error) = response.error {
        let detail = error
            .data
            .and_then(|data| data.details)
            .map(|details| format!(": {details}"))
            .unwrap_or_default();
        return Err(ClientError::Rpc(format!(
            "{} (code {}){detail}",
            error.message, error.code
        )));
    }
    Ok(response.result.unwrap_or(Value::Null))
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(limit).collect();
    truncated.push('…');
    truncated
}

async fn unix_subscribe(
    path: &Path,
    topics: Option<Vec<String>>,
    last_event_id: Option<u64>,
    timeout: Duration,
) -> Result<BoxStream<'static, Result<SseEvent, ClientError>>, ClientError> {
    let stream = UnixStream::connect(path).await.map_err(|error| {
        ClientError::Transport(format!("cannot connect to {}: {error}", path.display()))
    })?;
    let (reader, mut writer) = stream.into_split();
    let params = EventsSubscribeParams {
        topics,
        last_event_id,
    };
    let params = serde_json::to_value(params)
        .map_err(|error| ClientError::Protocol(format!("cannot encode params: {error}")))?;
    let request = RpcRequest::new(0_i64, methods::EVENTS_SUBSCRIBE, Some(params));
    let mut line = encode_request(&request)?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|error| ClientError::Transport(format!("cannot subscribe: {error}")))?;
    writer
        .flush()
        .await
        .map_err(|error| ClientError::Transport(format!("cannot flush subscribe: {error}")))?;

    let mut lines = BufReader::new(reader).lines();
    let response_line = tokio::time::timeout(timeout, lines.next_line())
        .await
        .map_err(|_| ClientError::Timeout(timeout))?
        .map_err(|error| ClientError::Transport(format!("cannot read subscribe reply: {error}")))?
        .ok_or_else(|| {
            ClientError::Disconnected("daemon closed before acknowledging subscribe".to_owned())
        })?;
    let response: RpcResponse = serde_json::from_str(response_line.trim())
        .map_err(|error| ClientError::Protocol(format!("invalid subscribe reply: {error}")))?;
    let result = response_into_result(response)?;
    if result
        .get("subscription_id")
        .and_then(Value::as_str)
        .is_none()
    {
        return Err(ClientError::Protocol(
            "events.subscribe reply did not contain subscription_id".to_owned(),
        ));
    }

    let stream = async_stream::stream! {
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.len() > MAX_LINE_BYTES {
                        yield Err(ClientError::Protocol("event line is too large".to_owned()));
                        break;
                    }
                    match events::decode_jsonl_event(&line) {
                        Ok(Some(event)) => yield Ok(event),
                        Ok(None) => {}
                        Err(error) => yield Err(error),
                    }
                }
                Ok(None) => {
                    yield Err(ClientError::Disconnected("event stream closed".to_owned()));
                    break;
                }
                Err(error) => {
                    yield Err(ClientError::Transport(format!("event read failed: {error}")));
                    break;
                }
            }
        }
    };
    Ok(Box::pin(stream))
}

async fn http_subscribe(
    http: reqwest::Client,
    base: &str,
    last_event_id: Option<u64>,
) -> Result<BoxStream<'static, Result<SseEvent, ClientError>>, ClientError> {
    let url = format!("{base}/events");
    let mut request = http.get(&url);
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id.to_string());
    }
    let response = request
        .send()
        .await
        .map_err(|error| ClientError::Transport(format!("GET {url} failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ClientError::HttpStatus {
            status: status.as_u16(),
            body: "SSE stream request failed".to_owned(),
        });
    }
    let mut chunks = response.bytes_stream();
    let stream = async_stream::stream! {
        let mut buffer = String::new();
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    if buffer.len() > MAX_LINE_BYTES {
                        yield Err(ClientError::Protocol("SSE buffer exceeded limit".to_owned()));
                        break;
                    }
                    while let Some(frame) = events::take_frame(&mut buffer) {
                        match events::decode_frame(&frame) {
                            Ok(Some(event)) => yield Ok(event),
                            Ok(None) => {}
                            Err(error) => yield Err(error),
                        }
                    }
                }
                Err(error) => {
                    yield Err(ClientError::Transport(format!("SSE stream failed: {error}")));
                    break;
                }
            }
        }
        yield Err(ClientError::Disconnected("SSE stream closed".to_owned()));
    };
    Ok(Box::pin(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::rpc::DaemonStatus;
    use knect_proto::{RpcId, RpcResponse};
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    async fn mock_daemon(
        path: std::path::PathBuf,
        seen: tokio::sync::mpsc::UnboundedSender<RpcRequest>,
    ) {
        let listener = UnixListener::bind(&path).expect("bind");
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let request: RpcRequest = serde_json::from_str(&line).expect("request");
                    let response = if request.method == methods::EVENTS_SUBSCRIBE {
                        RpcResponse::success(
                            request.id.clone(),
                            json!({"subscription_id": "sub-1"}),
                        )
                    } else {
                        RpcResponse::success(
                            request.id.clone(),
                            json!({
                                "version": "0.1.0",
                                "schema_version": 1,
                                "context": "fixture",
                                "generation": 1,
                                "watchers": [],
                                "uptime_secs": 1,
                                "actions_enabled": false,
                                "tap_count": 0,
                                "started_at": null,
                                "cache_enabled": true,
                                "last_resync": null
                            }),
                        )
                    };
                    let _ = seen.send(request);
                    writer
                        .write_all(
                            format!("{}\n", serde_json::to_string(&response).expect("encode"))
                                .as_bytes(),
                        )
                        .await
                        .expect("write");
                    writer.flush().await.expect("flush");
                }
            });
        }
    }

    #[test]
    fn response_errors_keep_the_code_and_details() {
        let response = RpcResponse::failure(
            RpcId::Number(1),
            knect_proto::RpcError::with_data(
                knect_proto::error::NOT_FOUND,
                "missing",
                knect_proto::ErrorData {
                    retryable: false,
                    details: Some("gone".to_owned()),
                    resource: None,
                },
            ),
        );
        match response_into_result(response) {
            Err(ClientError::Rpc(message)) => {
                assert!(message.contains("missing"));
                assert!(message.contains("-32001"));
                assert!(message.contains("gone"));
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn truncate_is_utf8_safe() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }

    #[tokio::test]
    async fn unix_calls_increment_ids_and_decode_results() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("knectd.sock");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let server_path = path.clone();
        let server = tokio::spawn(async move { mock_daemon(server_path, tx).await });
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let client = RpcClientImpl::new(Endpoint::Unix(path), Duration::from_secs(2));
        let first = client
            .call(methods::DAEMON_STATUS.to_owned(), Some(json!({})))
            .await
            .expect("first call");
        let status: DaemonStatus = serde_json::from_value(first).expect("status");
        assert_eq!(status.context, "fixture");
        let _ = client
            .call(methods::DAEMON_STATUS.to_owned(), Some(json!({})))
            .await
            .expect("second call");
        let one = rx.recv().await.expect("first request");
        let two = rx.recv().await.expect("second request");
        assert_eq!(one.id, RpcId::Number(1));
        assert_eq!(two.id, RpcId::Number(2));
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn unix_subscribe_decodes_jsonl_events() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("knectd.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut lines = BufReader::new(reader).lines();
            let line = lines
                .next_line()
                .await
                .expect("read")
                .expect("subscribe request");
            let request: RpcRequest = serde_json::from_str(&line).expect("request");
            assert_eq!(request.method, methods::EVENTS_SUBSCRIBE);
            let response =
                RpcResponse::success(request.id.clone(), json!({"subscription_id": "sub-1"}));
            writer
                .write_all(
                    format!("{}\n", serde_json::to_string(&response).expect("response")).as_bytes(),
                )
                .await
                .expect("write response");
            let event = SseEvent::new(
                7,
                chrono::Utc::now(),
                knect_proto::events::SseEventKind::Heartbeat {
                    data: knect_proto::events::Heartbeat {
                        uptime_secs: 1,
                        generation: 2,
                        node_count: 3,
                        edge_count: 4,
                        ts: chrono::Utc::now(),
                    },
                },
            );
            writer
                .write_all(
                    format!("{}\n", serde_json::to_string(&event).expect("event")).as_bytes(),
                )
                .await
                .expect("write event");
            writer.flush().await.expect("flush");
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let client = RpcClientImpl::new(Endpoint::Unix(path), Duration::from_secs(2));
        let mut stream = client.subscribe(None, None).await.expect("subscribe");
        let event = stream.next().await.expect("event").expect("decoded event");
        assert_eq!(event.seq, 7);
        assert_eq!(event.event_type(), "daemon.heartbeat");
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn call_timeouts_are_typed() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("knectd.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let client = RpcClientImpl::new(Endpoint::Unix(path), Duration::from_millis(50));
        let error = client
            .call(methods::DAEMON_STATUS.to_owned(), None)
            .await
            .expect_err("timeout");
        assert!(matches!(error, ClientError::Timeout(_)));
        server.abort();
        let _ = server.await;
    }
}
