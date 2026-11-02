//! Daemon client boundary.
//!
//! The trait and endpoint resolution are stable; [`RpcClientImpl`] implements
//! the frozen JSON-RPC contract over newline-delimited Unix sockets and HTTP.

pub mod events;
pub mod rpc;
pub mod spawn;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use knect_proto::SseEvent;
use serde_json::Value;

pub use rpc::RpcClientImpl;
pub use spawn::{SpawnedDaemon, locate_daemon, locate_daemon_with, spawn_daemon, wait_for_socket};

/// Default daemon socket path.
pub const DEFAULT_SOCKET: &str = "/tmp/knectd.sock";

/// Default deadline for one RPC call.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Default deadline for a spawned daemon to start serving.
pub const DEFAULT_SPAWN_TIMEOUT: Duration = Duration::from_secs(8);

/// Endpoint selected for a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket.
    Unix(PathBuf),
    /// HTTP base URL.
    Http(String),
}

impl Endpoint {
    /// Human-readable endpoint label used in the status bar.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Unix(path) => path.display().to_string(),
            Self::Http(url) => url.clone(),
        }
    }

    /// True when this is the implicit default Unix socket.
    #[must_use]
    pub fn is_default_socket(&self) -> bool {
        matches!(self, Self::Unix(path) if path == Path::new(DEFAULT_SOCKET))
    }
}

/// Client failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Endpoint input was invalid.
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    /// Transport failed.
    #[error("transport error: {0}")]
    Transport(String),
    /// The daemon returned a JSON-RPC error.
    #[error("rpc error: {0}")]
    Rpc(String),
    /// A call exceeded its deadline.
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    /// The daemon closed a connection unexpectedly.
    #[error("daemon disconnected: {0}")]
    Disconnected(String),
    /// The wire payload did not match the frozen contract.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The daemon could not be launched.
    #[error("cannot spawn knectd: {0}")]
    Spawn(String),
    /// The HTTP endpoint answered with a non-success status.
    #[error("daemon returned HTTP {status}: {body}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Short body preview.
        body: String,
    },
}

/// Resolves an explicit HTTP URL or a Unix socket path.
///
/// `daemon_url` wins when both are supplied. A URL without a scheme is assumed
/// to be HTTP.
pub fn resolve_endpoint(
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<Endpoint, ClientError> {
    if let Some(url) = daemon_url {
        let url = url.trim();
        if url.is_empty() {
            return Err(ClientError::InvalidEndpoint(
                "daemon URL is empty".to_owned(),
            ));
        }
        let has_scheme = url.contains("://");
        let candidate = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_owned()
        } else if has_scheme {
            return Err(ClientError::InvalidEndpoint(format!(
                "'{url}' must use http or https"
            )));
        } else {
            format!("http://{url}")
        };
        let parsed = reqwest::Url::parse(&candidate)
            .map_err(|error| ClientError::InvalidEndpoint(format!("'{url}': {error}")))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(ClientError::InvalidEndpoint(format!(
                "'{url}' must use http or https"
            )));
        }
        if parsed.host_str().is_none() {
            return Err(ClientError::InvalidEndpoint(format!("'{url}' has no host")));
        }
        return Ok(Endpoint::Http(
            parsed.as_str().trim_end_matches('/').to_owned(),
        ));
    }
    Ok(Endpoint::Unix(
        socket
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET)),
    ))
}

/// JSON-RPC client used by the TUI and CLI.
pub trait RpcClient: Send + Sync {
    /// Calls one method.
    fn call(
        &self,
        method: String,
        params: Option<Value>,
    ) -> BoxFuture<'_, Result<Value, ClientError>>;

    /// Subscribes to daemon events.
    fn subscribe(
        &self,
        topics: Option<Vec<String>>,
        last_event_id: Option<u64>,
    ) -> BoxFuture<'_, Result<BoxStream<'static, Result<SseEvent, ClientError>>, ClientError>>;
}

/// How the CLI asked us to connect.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Resolved endpoint.
    pub endpoint: Endpoint,
    /// True when the user supplied `--socket` or `--daemon-url`.
    pub explicit: bool,
    /// Allow auto-spawning `knectd` when the endpoint is unreachable.
    pub allow_spawn: bool,
    /// Deadline for one call.
    pub call_timeout: Duration,
    /// Deadline for a spawned daemon to start serving.
    pub spawn_timeout: Duration,
}

impl ConnectOptions {
    /// Builds options with production defaults.
    #[must_use]
    pub fn new(endpoint: Endpoint, explicit: bool) -> Self {
        Self {
            endpoint,
            explicit,
            allow_spawn: !explicit,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            spawn_timeout: DEFAULT_SPAWN_TIMEOUT,
        }
    }
}

/// Result of a connection attempt.
pub struct Connected {
    /// Client bound to the resolved endpoint.
    pub client: Arc<dyn RpcClient>,
    /// Endpoint used by the client.
    pub endpoint: Endpoint,
    /// Handle to a daemon this process spawned, if any.
    pub spawned: Option<SpawnedDaemon>,
    /// Warning to surface in the status bar when the endpoint was unreachable.
    pub warning: Option<String>,
    /// True when the endpoint answered a health check.
    pub reachable: bool,
}

/// Connects, optionally spawning the daemon, and never returns an error for an
/// unreachable endpoint: the TUI stays usable and keeps retrying.
pub async fn connect(options: ConnectOptions) -> Result<Connected, ClientError> {
    let reachable = is_reachable(&options.endpoint, Duration::from_millis(500)).await;
    let mut spawned = None;
    let mut warning = None;
    if !reachable && options.allow_spawn && !options.explicit {
        match &options.endpoint {
            Endpoint::Unix(path) => match locate_daemon(None) {
                Some(program) => match spawn_daemon(&program, path).await {
                    Ok(handle) => {
                        if wait_for_socket(path, options.spawn_timeout).await {
                            spawned = Some(handle);
                        } else {
                            let log = handle
                                .log_path()
                                .map(|path| format!("; daemon log: {}", path.display()))
                                .unwrap_or_default();
                            warning = Some(format!(
                                "spawned {} but {} did not become ready{log}",
                                program.display(),
                                path.display()
                            ));
                        }
                    }
                    Err(error) => warning = Some(error.to_string()),
                },
                None => {
                    warning = Some(format!(
                        "no daemon at {}; knectd was not found. Build it with `cargo build -p knectd`, install it, or set KNECTD_BIN",
                        path.display()
                    ));
                }
            },
            Endpoint::Http(url) => {
                warning = Some(format!("no daemon reachable at {url}"));
            }
        }
    } else if !reachable {
        warning = Some(format!(
            "no daemon reachable at {}; retrying",
            options.endpoint.describe()
        ));
    }
    let client: Arc<dyn RpcClient> = Arc::new(RpcClientImpl::new(
        options.endpoint.clone(),
        options.call_timeout,
    ));
    Ok(Connected {
        client,
        endpoint: options.endpoint,
        reachable: reachable || spawned.is_some(),
        spawned,
        warning,
    })
}

/// Probes an endpoint without performing a full RPC call.
pub async fn is_reachable(endpoint: &Endpoint, timeout: Duration) -> bool {
    match endpoint {
        Endpoint::Unix(path) => {
            tokio::time::timeout(timeout, tokio::net::UnixStream::connect(path))
                .await
                .map(|result| result.is_ok())
                .unwrap_or(false)
        }
        Endpoint::Http(base) => {
            crate::ensure_crypto_provider();
            let Ok(client) = reqwest::Client::builder().connect_timeout(timeout).build() else {
                return false;
            };
            let url = format!("{base}/healthz");
            tokio::time::timeout(timeout, client.get(&url).send())
                .await
                .ok()
                .and_then(Result::ok)
                .is_some_and(|response| response.status().is_success())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_prefers_http_and_normalizes_scheme() {
        assert_eq!(
            resolve_endpoint(Some(Path::new("/tmp/a.sock")), Some("127.0.0.1:7420/"))
                .expect("endpoint"),
            Endpoint::Http("http://127.0.0.1:7420".to_owned())
        );
        assert_eq!(
            resolve_endpoint(None, Some("http://127.0.0.1:7420/")).expect("endpoint"),
            Endpoint::Http("http://127.0.0.1:7420".to_owned())
        );
        assert_eq!(
            resolve_endpoint(None, Some("https://example.test/knect")).expect("endpoint"),
            Endpoint::Http("https://example.test/knect".to_owned())
        );
    }

    #[test]
    fn endpoint_defaults_to_the_standard_socket() {
        assert_eq!(
            resolve_endpoint(None, None).expect("endpoint"),
            Endpoint::Unix(PathBuf::from(DEFAULT_SOCKET))
        );
        assert_eq!(
            resolve_endpoint(Some(Path::new("/tmp/custom.sock")), None).expect("endpoint"),
            Endpoint::Unix(PathBuf::from("/tmp/custom.sock"))
        );
    }

    #[test]
    fn empty_or_malformed_url_is_invalid() {
        assert!(matches!(
            resolve_endpoint(None, Some("   ")),
            Err(ClientError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            resolve_endpoint(None, Some("ftp://example.test")),
            Err(ClientError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            resolve_endpoint(None, Some("http://")),
            Err(ClientError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn endpoint_labels_are_stable() {
        assert_eq!(
            Endpoint::Unix(PathBuf::from(DEFAULT_SOCKET)).describe(),
            DEFAULT_SOCKET
        );
        assert!(Endpoint::Unix(PathBuf::from(DEFAULT_SOCKET)).is_default_socket());
        assert!(!Endpoint::Http("http://127.0.0.1:7420".to_owned()).is_default_socket());
    }

    #[test]
    fn client_trait_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn RpcClient>();
        let client: Arc<dyn RpcClient> = Arc::new(RpcClientImpl::new(
            Endpoint::Unix(PathBuf::from(DEFAULT_SOCKET)),
            Duration::from_millis(100),
        ));
        assert!(Arc::strong_count(&client) == 1);
    }

    #[tokio::test]
    async fn explicit_endpoints_do_not_auto_spawn() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("missing.sock");
        let options = ConnectOptions::new(Endpoint::Unix(path), true);
        let connected = connect(options).await.expect("connect");
        assert!(!connected.reachable);
        assert!(connected.spawned.is_none());
        assert!(
            connected
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("retrying"))
        );
    }
}
