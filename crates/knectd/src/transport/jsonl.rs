//! Newline-delimited JSON-RPC framing for the Unix domain socket.
//!
//! One request object per line, one response object per line. Notifications
//! produce no line. Subscription streaming (`events.subscribe`) is a Task 5
//! concern; the foundation dispatcher returns a typed error for it.

use knect_proto::error::{INVALID_REQUEST, PARSE_ERROR};
use knect_proto::{RpcId, RpcRequest, RpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};

use crate::app::AppState;
use crate::error::DaemonError;
use crate::rpc::RpcDispatcher;

/// Serves JSONL requests until the peer closes the connection.
pub async fn serve_connection<S>(state: AppState, stream: S) -> Result<(), DaemonError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, writer) = tokio::io::split(stream);
    serve_io(state, reader, writer).await
}

/// Generic JSONL loop used by the UDS listener and unit tests.
pub async fn serve_io<R, W>(state: AppState, reader: R, writer: W) -> Result<(), DaemonError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut writer = BufWriter::new(writer);
    let dispatcher = RpcDispatcher::new(state);
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                return Err(DaemonError::Transport(format!(
                    "JSONL read failed: {error}"
                )));
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(response) = handle_line(&dispatcher, trimmed) else {
            continue;
        };
        let encoded = serde_json::to_string(&response)
            .map_err(|error| DaemonError::Rpc(format!("cannot encode response: {error}")))?;
        writer
            .write_all(encoded.as_bytes())
            .await
            .map_err(|error| DaemonError::Transport(format!("JSONL write failed: {error}")))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|error| DaemonError::Transport(format!("JSONL write failed: {error}")))?;
        writer
            .flush()
            .await
            .map_err(|error| DaemonError::Transport(format!("JSONL flush failed: {error}")))?;
    }
    Ok(())
}

/// Decodes and dispatches one JSONL line, returning the response line.
#[must_use]
pub fn handle_line(dispatcher: &RpcDispatcher, line: &str) -> Option<RpcResponse> {
    match serde_json::from_str::<RpcRequest>(line) {
        Ok(request) => dispatcher.dispatch(request),
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
    use tokio::io::duplex;

    #[test]
    fn handle_line_dispatches_and_reports_parse_errors() {
        let state = AppState::new(Config::default(), true);
        let dispatcher = RpcDispatcher::new(state);
        let response = handle_line(
            &dispatcher,
            r#"{"jsonrpc":"2.0","id":1,"method":"daemon.status","params":{}}"#,
        )
        .expect("response");
        assert!(response.error.is_none());
        let error = handle_line(&dispatcher, "{").expect("error response");
        assert_eq!(
            error.error.map(|error| error.code),
            Some(knect_proto::error::PARSE_ERROR)
        );
    }

    #[tokio::test]
    async fn serve_io_round_trips_one_request() {
        let state = AppState::new(Config::default(), true);
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
}
