//! Daemon client boundary.
//!
//! Foundation skeleton only: the trait and endpoint resolution are frozen; the
//! HTTP/UDS implementation lands with Tasks 13-15.

use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use knect_proto::SseEvent;
use serde_json::Value;

/// Default daemon socket path.
pub const DEFAULT_SOCKET: &str = "/tmp/knectd.sock";

/// Endpoint selected for a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket.
    Unix(PathBuf),
    /// HTTP base URL.
    Http(String),
}

/// Client failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The operation is not implemented in this build.
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),
    /// Endpoint input was invalid.
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    /// Transport failed.
    #[error("transport error: {0}")]
    Transport(String),
    /// The daemon returned a JSON-RPC error.
    #[error("rpc error: {0}")]
    Rpc(String),
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
        if url.starts_with("http://") || url.starts_with("https://") {
            return Ok(Endpoint::Http(url.trim_end_matches('/').to_owned()));
        }
        return Ok(Endpoint::Http(format!(
            "http://{}",
            url.trim_end_matches('/')
        )));
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

/// Client placeholder that returns typed errors.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnimplementedClient;

impl RpcClient for UnimplementedClient {
    fn call(
        &self,
        _method: String,
        _params: Option<Value>,
    ) -> BoxFuture<'_, Result<Value, ClientError>> {
        Box::pin(async move { Err(ClientError::NotImplemented("RPC call")) })
    }

    fn subscribe(
        &self,
        _topics: Option<Vec<String>>,
        _last_event_id: Option<u64>,
    ) -> BoxFuture<'_, Result<BoxStream<'static, Result<SseEvent, ClientError>>, ClientError>> {
        Box::pin(async move { Err(ClientError::NotImplemented("event subscription")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

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
    fn empty_url_is_invalid() {
        assert!(matches!(
            resolve_endpoint(None, Some("   ")),
            Err(ClientError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn unimplemented_client_returns_typed_errors() {
        let client = UnimplementedClient;
        let error = client
            .call("daemon.status".to_owned(), None)
            .now_or_never()
            .expect("ready")
            .expect_err("unimplemented");
        assert!(matches!(error, ClientError::NotImplemented(_)));
        match client.subscribe(None, None).now_or_never().expect("ready") {
            Err(ClientError::NotImplemented(_)) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("unexpected successful subscription"),
        }
    }

    #[test]
    fn client_trait_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn RpcClient>();
        let _client: std::sync::Arc<dyn RpcClient> = std::sync::Arc::new(UnimplementedClient);
    }
}
