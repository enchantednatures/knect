//! HTTP and UDS JSON-RPC harness helpers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use knect_proto::events::SseEvent;
use knect_proto::graph::Topology;
use knect_proto::rpc::JSONRPC_VERSION;
use knect_proto::{RpcError, RpcId, RpcRequest, RpcResponse, SCHEMA_VERSION};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Default daemon socket path.
pub const DEFAULT_SOCKET: &str = "/tmp/knectd.sock";
static REQUEST_ID: AtomicI64 = AtomicI64::new(1);
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Harness failure.
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// Endpoint input was invalid.
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    /// Transport failure.
    #[error("transport error: {0}")]
    Transport(String),
    /// JSON-RPC error response.
    #[error("rpc error: {0}")]
    Rpc(String),
    /// Protocol payload did not match the contract.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A wait deadline expired.
    #[error("timed out waiting for {0}")]
    Timeout(String),
}

impl HarnessError {
    /// Wraps a daemon RPC error with its code and message.
    #[must_use]
    pub fn from_rpc(error: RpcError) -> Self {
        Self::Rpc(format!("{} ({})", error.message, error.code))
    }
}

/// Selected transport.
#[derive(Debug)]
enum Transport {
    Http {
        base: String,
        client: reqwest::Client,
    },
    Unix {
        path: PathBuf,
    },
}

/// RPC and assertion harness.
#[derive(Debug)]
pub struct Harness {
    transport: Transport,
}

impl Harness {
    /// Builds a harness. `daemon_url` wins over `socket`; the default socket is
    /// used when neither is supplied.
    pub fn new(daemon_url: Option<&str>, socket: Option<&Path>) -> Result<Self, HarnessError> {
        if let Some(url) = daemon_url {
            let url = url.trim();
            if url.is_empty() {
                return Err(HarnessError::InvalidEndpoint(
                    "daemon URL is empty".to_owned(),
                ));
            }
            let base = if url.starts_with("http://") || url.starts_with("https://") {
                url.trim_end_matches('/').to_owned()
            } else {
                format!("http://{}", url.trim_end_matches('/'))
            };
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| HarnessError::Transport(error.to_string()))?;
            return Ok(Self {
                transport: Transport::Http { base, client },
            });
        }
        let path = socket
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET));
        Ok(Self {
            transport: Transport::Unix { path },
        })
    }

    /// Calls one JSON-RPC method and returns its result.
    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, HarnessError> {
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let request = RpcRequest::new(id, method, params);
        let response = match &self.transport {
            Transport::Http { base, client } => {
                let response = client
                    .post(format!("{base}/rpc"))
                    .json(&request)
                    .send()
                    .await
                    .map_err(|error| HarnessError::Transport(error.to_string()))?;
                response
                    .json::<RpcResponse>()
                    .await
                    .map_err(|error| HarnessError::Protocol(error.to_string()))?
            }
            Transport::Unix { path } => call_unix(path, &request).await?,
        };
        if let Some(error) = response.error {
            return Err(HarnessError::from_rpc(error));
        }
        response
            .result
            .ok_or_else(|| HarnessError::Protocol(format!("method {method} returned no result")))
    }

    /// Fetches and validates the current topology.
    pub async fn topology(&self) -> Result<Topology, HarnessError> {
        let value = self
            .call("graph.get_topology", Some(serde_json::json!({})))
            .await?;
        let topology: Topology = serde_json::from_value(value)
            .map_err(|error| HarnessError::Protocol(format!("invalid topology: {error}")))?;
        assert_topology(&topology)?;
        Ok(topology)
    }

    /// Waits until `check` returns true or the deadline expires.
    pub async fn wait_for<F, Fut>(
        &self,
        label: &str,
        timeout: Duration,
        interval: Duration,
        mut check: F,
    ) -> Result<(), HarnessError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<bool, HarnessError>>,
    {
        let deadline = Instant::now() + timeout;
        loop {
            if check().await? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::Timeout(label.to_owned()));
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Waits until the daemon reports at least `generation`.
    pub async fn wait_for_generation(&self, generation: u64) -> Result<(), HarnessError> {
        self.wait_for(
            "topology generation",
            Duration::from_secs(10),
            Duration::from_millis(50),
            || async move {
                Ok(self
                    .call("daemon.status", Some(serde_json::json!({})))
                    .await?["generation"]
                    .as_u64()
                    .unwrap_or(0)
                    >= generation)
            },
        )
        .await
    }
}

/// Validates schema version, counts, and deterministic ordering.
pub fn assert_topology(topology: &Topology) -> Result<(), HarnessError> {
    if topology.schema_version != SCHEMA_VERSION {
        return Err(HarnessError::Protocol(format!(
            "schema version {} != {SCHEMA_VERSION}",
            topology.schema_version
        )));
    }
    if topology.stats.node_count != topology.nodes.len() as u64 {
        return Err(HarnessError::Protocol(
            "stats.node_count does not match nodes".to_owned(),
        ));
    }
    if topology.stats.edge_count != topology.edges.len() as u64 {
        return Err(HarnessError::Protocol(
            "stats.edge_count does not match edges".to_owned(),
        ));
    }
    if !topology
        .nodes
        .windows(2)
        .all(|pair| pair[0].id <= pair[1].id)
    {
        return Err(HarnessError::Protocol(
            "nodes are not ordered by ResourceId".to_owned(),
        ));
    }
    if !topology
        .edges
        .windows(2)
        .all(|pair| pair[0].id <= pair[1].id)
    {
        return Err(HarnessError::Protocol(
            "edges are not ordered by edge ID".to_owned(),
        ));
    }
    Ok(())
}

/// Asserts that an event carries the expected frozen event name.
pub fn assert_event(event: &SseEvent, expected: &str) -> Result<(), HarnessError> {
    if event.event_type() != expected {
        return Err(HarnessError::Protocol(format!(
            "expected event {expected}, got {}",
            event.event_type()
        )));
    }
    Ok(())
}

/// Returns a unique temporary socket path for a scenario.
#[must_use]
pub fn temp_socket_path(label: &str) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "knect-e2e-{}-{label}-{counter}.sock",
        std::process::id()
    ))
}

/// Returns a currently free loopback port.
pub fn free_port() -> Result<u16, HarnessError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| HarnessError::Transport(format!("cannot reserve port: {error}")))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| HarnessError::Transport(format!("cannot read port: {error}")))
}

async fn call_unix(path: &Path, request: &RpcRequest) -> Result<RpcResponse, HarnessError> {
    let stream = UnixStream::connect(path).await.map_err(|error| {
        HarnessError::Transport(format!("cannot connect to {}: {error}", path.display()))
    })?;
    let (reader, mut writer) = stream.into_split();
    let line = serde_json::to_string(request)
        .map_err(|error| HarnessError::Protocol(format!("cannot encode request: {error}")))?;
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|error| HarnessError::Transport(error.to_string()))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| HarnessError::Transport(error.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|error| HarnessError::Transport(error.to_string()))?;
    let mut lines = BufReader::new(reader).lines();
    let response = lines
        .next_line()
        .await
        .map_err(|error| HarnessError::Transport(error.to_string()))?
        .ok_or_else(|| HarnessError::Protocol("daemon closed the socket".to_owned()))?;
    let response: RpcResponse = serde_json::from_str(&response)
        .map_err(|error| HarnessError::Protocol(error.to_string()))?;
    if response.jsonrpc != JSONRPC_VERSION {
        return Err(HarnessError::Protocol(format!(
            "unexpected jsonrpc version {}",
            response.jsonrpc
        )));
    }
    Ok(response)
}

/// Builds a JSON-RPC request for scenario code.
#[must_use]
pub fn request(method: &str, params: Option<Value>) -> RpcRequest {
    RpcRequest::new(RpcId::Number(1), method, params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::events::{Heartbeat, SseEventKind};

    #[test]
    fn harness_endpoint_resolution_is_explicit() {
        let harness = Harness::new(None, None).expect("harness");
        assert!(matches!(harness.transport, Transport::Unix { .. }));
        let harness =
            Harness::new(Some("127.0.0.1:7420"), Some(Path::new("/tmp/x"))).expect("harness");
        match harness.transport {
            Transport::Http { base, .. } => assert_eq!(base, "http://127.0.0.1:7420"),
            Transport::Unix { .. } => panic!("expected HTTP transport"),
        }
        assert!(Harness::new(Some("  "), None).is_err());
    }

    #[test]
    fn topology_validation_accepts_empty_and_rejects_bad_counts() {
        let mut topology = Topology {
            schema_version: SCHEMA_VERSION,
            context: "fixture".to_owned(),
            generation: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            warnings: Vec::new(),
            stats: Default::default(),
        };
        assert!(assert_topology(&topology).is_ok());
        topology.stats.node_count = 1;
        assert!(matches!(
            assert_topology(&topology),
            Err(HarnessError::Protocol(_))
        ));
    }

    #[test]
    fn event_assertion_checks_the_frozen_name() {
        let event = SseEvent::new(
            1,
            chrono::Utc::now(),
            SseEventKind::Heartbeat {
                data: Heartbeat {
                    uptime_secs: 1,
                    generation: 0,
                    node_count: 0,
                    edge_count: 0,
                    ts: chrono::Utc::now(),
                },
            },
        );
        assert!(assert_event(&event, "daemon.heartbeat").is_ok());
        assert!(assert_event(&event, "snapshot").is_err());
    }

    #[test]
    fn temp_paths_and_ports_are_unique() {
        let first = temp_socket_path("test");
        let second = temp_socket_path("test");
        assert_ne!(first, second);
        let port = free_port().expect("free port");
        assert!(port > 0);
    }
}
