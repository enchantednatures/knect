//! HTTP transport: `POST /rpc`, `GET /events`, `GET /healthz`, static fallback.

use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use knect_proto::error::{INVALID_REQUEST, PARSE_ERROR};
use knect_proto::{RpcId, RpcRequest, RpcResponse};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::app::AppState;
use crate::error::DaemonError;
use crate::rpc::RpcDispatcher;
use crate::transport::sse;
use crate::webui;

/// Builds the shared HTTP/UDS router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/rpc", post(rpc_handler))
        .route("/events", get(events_handler))
        .route("/healthz", get(healthz_handler))
        .fallback(webui::fallback)
        .with_state(state)
}

/// Serves HTTP until cancellation.
pub async fn serve(
    state: AppState,
    address: String,
    cancel: CancellationToken,
) -> Result<(), DaemonError> {
    let address: SocketAddr = address.parse().map_err(|error| {
        DaemonError::Config(format!("invalid daemon.http_addr '{address}': {error}"))
    })?;
    if !address.ip().is_loopback() && !state.config.daemon.allow_remote {
        return Err(DaemonError::Config(format!(
            "refusing non-loopback bind {address}; set daemon.allow_remote = true or KNECT_ALLOW_REMOTE=1"
        )));
    }
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| DaemonError::Transport(format!("cannot bind {address}: {error}")))?;
    let bound = listener
        .local_addr()
        .map_err(|error| DaemonError::Transport(format!("cannot inspect {address}: {error}")))?;
    tracing::info!(address = %bound, "HTTP listener started");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .map_err(|error| DaemonError::Transport(format!("HTTP server failed: {error}")))
}

async fn rpc_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let request = match parse_request(&body) {
        Ok(request) => request,
        Err((code, message)) => {
            return Json(RpcResponse::failure(
                RpcId::Null,
                knect_proto::RpcError::new(code, message),
            ))
            .into_response();
        }
    };
    match RpcDispatcher::new(state).dispatch_async(request).await {
        Some(response) => Json(response).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn events_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let stream = sse::event_stream(state, last_event_id);
    let mut response = Sse::new(stream).into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

async fn healthz_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

fn parse_request(body: &[u8]) -> Result<RpcRequest, (i32, String)> {
    match serde_json::from_slice::<RpcRequest>(body) {
        Ok(request) => Ok(request),
        Err(error) => {
            let first = body.iter().find(|byte| !byte.is_ascii_whitespace());
            if first == Some(&b'[') {
                Err((
                    INVALID_REQUEST,
                    "batch requests are not supported; send one JSON-RPC object".to_owned(),
                ))
            } else if serde_json::from_slice::<serde_json::Value>(body).is_ok() {
                Err((
                    INVALID_REQUEST,
                    format!("invalid JSON-RPC request envelope: {error}"),
                ))
            } else {
                Err((PARSE_ERROR, format!("invalid JSON: {error}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::header;
    use http_body_util::BodyExt;
    use knect_core::config::Config;
    use tower::ServiceExt;

    fn state() -> AppState {
        AppState::new(Config::default(), true)
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("JSON body")
    }

    fn rpc_request(method: &str, params: Option<serde_json::Value>) -> Request<Body> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        Request::builder()
            .method("POST")
            .uri("/rpc")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload.to_string()))
            .expect("request")
    }

    #[tokio::test]
    async fn healthz_returns_ok_status() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn rpc_handler_serves_daemon_status() {
        let response = router(state())
            .oneshot(rpc_request("daemon.status", Some(json!({}))))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert!(body.get("error").is_none_or(serde_json::Value::is_null));
        assert_eq!(body["result"]["context"], "fixture");
        assert_eq!(body["result"]["generation"], 0);
        assert_eq!(body["result"]["schema_version"], 1);
    }

    #[tokio::test]
    async fn invalid_json_is_a_parse_error() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rpc")
                    .body(Body::from("{"))
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], PARSE_ERROR);
    }

    #[tokio::test]
    async fn batch_arrays_are_rejected() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rpc")
                    .body(Body::from("[]"))
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], INVALID_REQUEST);
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let response = router(state())
            .oneshot(rpc_request("does.not.exist", None))
            .await
            .expect("response");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], knect_proto::error::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn notification_returns_no_content() {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "daemon.status",
            "params": {},
        });
        let response = router(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rpc")
                    .body(Body::from(payload.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn events_replays_after_last_event_id() {
        use knect_proto::events::{Heartbeat, SseEventKind};

        let state = state();
        let heartbeat = || SseEventKind::Heartbeat {
            data: Heartbeat {
                uptime_secs: 1,
                generation: 0,
                node_count: 0,
                edge_count: 0,
                ts: chrono::Utc::now(),
            },
        };
        let first = state.events.publish(heartbeat());
        let second = state.events.publish(heartbeat());
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .header("last-event-id", first.seq.to_string())
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let mut body = response.into_body();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
            .await
            .expect("frame before timeout")
            .expect("body frame")
            .expect("data frame");
        let data = frame.into_data().expect("data frame payload");
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains(&format!("id: {}", second.seq)));
        assert!(!text.contains(&format!("id: {}", first.seq)));
    }

    #[tokio::test]
    async fn events_sets_sse_headers() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.starts_with("text/event-stream"));
    }

    #[tokio::test]
    async fn webui_root_serves_embedded_app() {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let html = String::from_utf8_lossy(&bytes);
        assert!(html.contains("id=\"app\""));
        assert!(!html.contains("WebUI not built"));
    }
}
