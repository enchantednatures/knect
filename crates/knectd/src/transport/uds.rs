//! Unix domain socket listener supporting HTTP/1.1 and JSONL framing.
//!
//! The socket is mode `0600`. A guard removes it on graceful shutdown. On Linux
//! the peer's UID must equal the socket file owner's UID; other peers are
//! rejected before any request is read.

use std::path::{Path, PathBuf};

use axum::Router;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::app::AppState;
use crate::error::DaemonError;
use crate::transport::http::router;
use crate::transport::jsonl;

/// Binds a UDS listener, replacing a stale socket and setting mode `0600`.
pub async fn bind_socket(path: &Path) -> Result<UnixListener, DaemonError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| {
            DaemonError::Transport(format!(
                "cannot remove stale socket {}: {error}",
                path.display()
            ))
        })?;
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            DaemonError::Transport(format!(
                "cannot create socket directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    let listener = UnixListener::bind(path).map_err(|error| {
        DaemonError::Transport(format!("cannot bind {}: {error}", path.display()))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            DaemonError::Transport(format!(
                "cannot set permissions on {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(listener)
}

/// Removes a socket path when it exists.
pub fn remove_socket(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// Serves the UDS listener until cancellation, then removes the socket.
pub async fn serve(
    state: AppState,
    path: PathBuf,
    cancel: CancellationToken,
) -> Result<(), DaemonError> {
    let listener = bind_socket(&path).await?;
    #[cfg(target_os = "linux")]
    let owner_uid = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&path).ok().map(|metadata| metadata.uid())
    };
    let guard = SocketGuard { path };
    tracing::info!(path = %guard.path.display(), "UDS listener started");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, _address) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!(%error, "UDS accept failed");
                        continue;
                    }
                };
                #[cfg(target_os = "linux")]
                {
                    let peer = stream.peer_cred().ok().map(|cred| cred.uid());
                    if !peer_uid_allowed(peer, owner_uid) {
                        tracing::warn!(
                            ?peer,
                            ?owner_uid,
                            "rejecting UDS peer with a different UID"
                        );
                        continue;
                    }
                }
                let state = state.clone();
                let router = router(state.clone());
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(state, router, stream).await {
                        tracing::debug!(%error, "UDS connection closed with error");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_connection(
    state: AppState,
    router: Router,
    stream: UnixStream,
) -> Result<(), DaemonError> {
    let mut buffered = BufReader::with_capacity(8 * 1024, stream);
    let is_jsonl = match buffered.fill_buf().await {
        Ok(buffer) => buffer
            .first()
            .is_some_and(|byte| *byte == b'{' || byte.is_ascii_whitespace()),
        Err(_) => false,
    };
    if is_jsonl {
        jsonl::serve_connection(state, buffered).await
    } else {
        serve_http(router, buffered).await
    }
}

async fn serve_http<S>(router: Router, stream: S) -> Result<(), DaemonError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request: hyper::Request<Incoming>| {
        let router = router.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = axum::body::Body::new(body);
            router
                .oneshot(hyper::Request::from_parts(parts, body))
                .await
        }
    });
    http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
        .map_err(|error| DaemonError::Transport(format!("UDS HTTP connection failed: {error}")))
}

/// Returns true when the peer UID matches the socket owner.
///
/// A missing peer credential or owner UID is rejected: the daemon must be able
/// to prove the peer is the same local user before serving it.
#[cfg(target_os = "linux")]
fn peer_uid_allowed(peer_uid: Option<u32>, owner_uid: Option<u32>) -> bool {
    matches!((peer_uid, owner_uid), (Some(peer), Some(owner)) if peer == owner)
}

struct SocketGuard {
    path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        remove_socket(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_core::config::Config;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[cfg(target_os = "linux")]
    #[test]
    fn peer_uid_must_match_the_socket_owner() {
        assert!(peer_uid_allowed(Some(1000), Some(1000)));
        assert!(!peer_uid_allowed(Some(1001), Some(1000)));
        assert!(!peer_uid_allowed(None, Some(1000)));
        assert!(!peer_uid_allowed(Some(1000), None));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn socket_is_private_and_stale_files_are_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("knectd.sock");
        std::fs::write(&path, b"stale").expect("stale file");
        let listener = bind_socket(&path).await.expect("bind");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        drop(listener);
        remove_socket(&path);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn serve_removes_socket_on_cancel_and_answers_jsonl() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("knectd.sock");
        let state = AppState::new(Config::default(), true);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(state, path.clone(), cancel.clone()));

        let mut connected = None;
        for _ in 0..100 {
            match UnixStream::connect(&path).await {
                Ok(stream) => {
                    connected = Some(stream);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
        let stream = connected.expect("socket accepts connections");
        let (reader, mut writer) = stream.into_split();
        writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"graph.get_topology\",\"params\":{}}\n",
            )
            .await
            .expect("write");
        writer.flush().await.expect("flush");
        let mut lines = BufReader::new(reader).lines();
        let line = lines.next_line().await.expect("read").expect("line");
        let response: knect_proto::RpcResponse =
            serde_json::from_str(&line).expect("response JSON");
        assert_eq!(response.id, knect_proto::RpcId::Number(3));
        assert!(response.error.is_none());
        let result = response.result.expect("result");
        assert_eq!(result["context"], "fixture");

        cancel.cancel();
        task.await.expect("join").expect("serve");
        assert!(!path.exists());
    }
}
