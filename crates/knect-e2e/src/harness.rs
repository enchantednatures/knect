//! HTTP and UDS JSON-RPC harness helpers.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
        crate::ensure_crypto_provider();
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

/// Returns the repository fixture directory.
#[must_use]
pub fn repo_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testdata/fixtures")
}

/// A fixture daemon spawned by the harness.
#[derive(Debug)]
pub struct FixtureDaemon {
    child: Child,
    socket: PathBuf,
    http_base: String,
    terminated: bool,
}

impl FixtureDaemon {
    /// Spawns `knectd --fixture <fixture>` on a temporary socket and free port.
    pub async fn spawn(fixture: &Path) -> Result<Self, HarnessError> {
        if !fixture.exists() {
            return Err(HarnessError::InvalidEndpoint(format!(
                "fixture directory {} does not exist",
                fixture.display()
            )));
        }
        let binary = knectd_binary().await?;
        let socket = temp_socket_path("fixture");
        let port = free_port()?;
        let http_addr = format!("127.0.0.1:{port}");
        let child = Command::new(&binary)
            .arg("--fixture")
            .arg(fixture)
            .arg("--socket-path")
            .arg(&socket)
            .arg("--http-addr")
            .arg(&http_addr)
            .arg("--log")
            .arg("warn")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                HarnessError::Transport(format!("cannot spawn {}: {error}", binary.display()))
            })?;
        let mut daemon = Self {
            child,
            socket,
            http_base: format!("http://{http_addr}"),
            terminated: false,
        };
        daemon.wait_until_ready().await?;
        Ok(daemon)
    }

    async fn wait_until_ready(&mut self) -> Result<(), HarnessError> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if UnixStream::connect(&self.socket).await.is_ok() {
                return Ok(());
            }
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| HarnessError::Transport(error.to_string()))?
            {
                return Err(HarnessError::Transport(format!(
                    "knectd exited before serving the socket: {status}"
                )));
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.terminated = true;
                return Err(HarnessError::Timeout(format!(
                    "fixture daemon socket {}",
                    self.socket.display()
                )));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Returns the daemon socket path.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Returns the HTTP base URL.
    #[must_use]
    pub fn http_base(&self) -> &str {
        &self.http_base
    }

    /// Sends `SIGTERM` and waits for the daemon to remove its socket.
    pub async fn shutdown(mut self) -> Result<(), HarnessError> {
        terminate(&mut self.child)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.socket.exists() {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.terminated = true;
                return Err(HarnessError::Timeout(format!(
                    "socket removal at {}",
                    self.socket.display()
                )));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let wait_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self
                .child
                .try_wait()
                .map_err(|error| HarnessError::Transport(error.to_string()))?
            {
                Some(_) => break,
                None if Instant::now() >= wait_deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                None => tokio::time::sleep(Duration::from_millis(25)).await,
            }
        }
        self.terminated = true;
        Ok(())
    }
}

impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if self.socket.exists() {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}

fn terminate(child: &mut Child) -> Result<(), HarnessError> {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status()
            .map_err(|error| HarnessError::Transport(format!("cannot signal knectd: {error}")))?;
        if !status.success() {
            return Err(HarnessError::Transport(format!(
                "kill -TERM {} exited with {status}",
                child.id()
            )));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        child
            .kill()
            .map_err(|error| HarnessError::Transport(format!("cannot kill knectd: {error}")))
    }
}

/// Returns the `knect` CLI binary, building it when absent.
pub async fn knect_binary() -> Result<PathBuf, HarnessError> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    let name = if cfg!(windows) { "knect.exe" } else { "knect" };
    let binary = target_dir.join("debug").join(name);
    if binary.is_file() {
        return Ok(binary);
    }
    let status = tokio::process::Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("knect-tui")
        .arg("--locked")
        .current_dir(&workspace)
        .status()
        .await
        .map_err(|error| HarnessError::Transport(format!("cannot run cargo build: {error}")))?;
    if !status.success() {
        return Err(HarnessError::Transport(format!(
            "cargo build -p knect-tui failed with {status}"
        )));
    }
    if binary.is_file() {
        return Ok(binary);
    }
    Err(HarnessError::Transport(format!(
        "built knect binary missing at {}",
        binary.display()
    )))
}

async fn knectd_binary() -> Result<PathBuf, HarnessError> {
    if let Ok(path) = std::env::var("KNECTD_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(HarnessError::InvalidEndpoint(format!(
            "KNECTD_BIN={} is not a file",
            path.display()
        )));
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    let binary = target_dir.join("debug").join(binary_name());
    if binary.is_file() && binary_is_fresh(&binary, &workspace) {
        return Ok(binary);
    }

    let status = tokio::process::Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("knectd")
        .arg("--locked")
        .current_dir(&workspace)
        .status()
        .await
        .map_err(|error| HarnessError::Transport(format!("cannot run cargo build: {error}")))?;
    if !status.success() {
        return Err(HarnessError::Transport(format!(
            "cargo build -p knectd failed with {status}"
        )));
    }
    if binary.is_file() {
        return Ok(binary);
    }
    if let Some(path) = sibling_binary() {
        return Ok(path);
    }
    Err(HarnessError::Transport(format!(
        "built knectd binary missing at {}",
        binary.display()
    )))
}

fn binary_is_fresh(binary: &Path, workspace: &Path) -> bool {
    let Ok(binary_time) = binary.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    let mut newest = binary_time;
    for directory in ["crates/knectd/src", "crates/knect-core/src"] {
        if let Some(modified) = newest_source_mtime(&workspace.join(directory)) {
            newest = newest.max(modified);
        }
    }
    binary_time >= newest
}

fn newest_source_mtime(directory: &Path) -> Option<std::time::SystemTime> {
    let mut newest = None;
    let entries = std::fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(modified) = newest_source_mtime(&path) {
                newest = Some(newest.map_or(modified, |current: std::time::SystemTime| {
                    current.max(modified)
                }));
            }
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && let Ok(modified) = path.metadata().and_then(|metadata| metadata.modified())
        {
            newest = Some(newest.map_or(modified, |current: std::time::SystemTime| {
                current.max(modified)
            }));
        }
    }
    newest
}

fn sibling_binary() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let directory = current.parent()?;
    [
        directory.join(binary_name()),
        directory.parent()?.join(binary_name()),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

const fn binary_name() -> &'static str {
    if cfg!(windows) {
        "knectd.exe"
    } else {
        "knectd"
    }
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
