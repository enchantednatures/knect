//! Token-guarded CloudEvent callback listener for live taps.
//!
//! One listener is bound per configured callback address. Every tap gets its
//! own path and token; the manager performs the constant-time token check and
//! CloudEvent parse, then this module publishes the resulting `TapEvent` on the
//! daemon SSE hub. The listener stops when the last tap goes away and is
//! explicitly stopped during daemon shutdown.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use knect_core::config::TapConfig;
use knect_core::error::CoreError;
use knect_core::tap::{TapCallbackRequest, TapDelivery, TapManager};
use knect_proto::events::SseEventKind;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::DaemonError;
use crate::transport::sse::EventHub;

const MAX_BODY_BYTES: usize = 1_048_576;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_VALUE: usize = 8_192;
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

static SERVER_COUNTER: AtomicU64 = AtomicU64::new(1);
static SERVERS: OnceLock<tokio::sync::Mutex<BTreeMap<String, std::sync::Arc<ServerHandle>>>> =
    OnceLock::new();

fn servers() -> &'static tokio::sync::Mutex<BTreeMap<String, std::sync::Arc<ServerHandle>>> {
    SERVERS.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()))
}

struct ServerHandle {
    id: u64,
    manager: std::sync::Arc<dyn TapManager>,
    cancel: CancellationToken,
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
struct CallbackState {
    manager: std::sync::Arc<dyn TapManager>,
    hub: std::sync::Arc<EventHub>,
}

/// Ensures a callback listener is bound for the configured address.
pub async fn ensure_for(
    config: &TapConfig,
    manager: std::sync::Arc<dyn TapManager>,
    hub: std::sync::Arc<EventHub>,
) -> Result<(), DaemonError> {
    let configured = config.callback_bind.trim();
    if configured.is_empty() {
        return Err(DaemonError::Config(
            "tap.callback_bind must not be empty".to_owned(),
        ));
    }

    let mut registry = servers().lock().await;
    if let Some(existing) = registry.get(configured).cloned() {
        let running = existing
            .task
            .lock()
            .await
            .as_ref()
            .is_some_and(|task| !task.is_finished());
        if running {
            return Ok(());
        }
        registry.remove(configured);
    }

    let listener = TcpListener::bind(configured).await.map_err(|error| {
        DaemonError::Transport(format!("cannot bind tap callback {configured}: {error}"))
    })?;
    let addr = listener.local_addr().map_err(|error| {
        DaemonError::Transport(format!("cannot read tap callback address: {error}"))
    })?;
    manager.set_callback_addr(addr);

    // Port 0 is a per-client ephemeral bind; key it by the concrete address so
    // parallel tests and two daemons on one host do not collide.
    let key = if configured.ends_with(":0") {
        addr.to_string()
    } else {
        configured.to_owned()
    };

    let id = SERVER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let cancel = CancellationToken::new();
    let state = CallbackState {
        manager: std::sync::Arc::clone(&manager),
        hub,
    };
    let router = Router::new()
        .route("/tap/{tap_id}/{token}", post(callback_handler))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state);

    let shutdown = cancel.clone();
    let task_cancel = cancel.clone();
    let reaper_manager = std::sync::Arc::clone(&manager);
    let task_key = key.clone();
    let task = tokio::spawn(async move {
        let server =
            axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned());
        let mut interval = tokio::time::interval(IDLE_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;

        tokio::select! {
            result = server => {
                if let Err(error) = result {
                    tracing::warn!(%error, "tap callback listener failed");
                }
            }
            _ = task_cancel.cancelled() => {}
            _ = async {
                loop {
                    interval.tick().await;
                    if let Err(error) = reaper_manager.reap().await {
                        tracing::warn!(%error, "tap reaper pass failed");
                    }
                    if reaper_manager.active_count() == 0 {
                        task_cancel.cancel();
                        break;
                    }
                }
            } => {}
        }

        let mut registry = servers().lock().await;
        if registry
            .get(&task_key)
            .is_some_and(|handle| handle.id == id)
        {
            registry.remove(&task_key);
        }
    });

    registry.insert(
        key,
        std::sync::Arc::new(ServerHandle {
            id,
            manager,
            cancel: cancel.clone(),
            task: tokio::sync::Mutex::new(Some(task)),
        }),
    );
    Ok(())
}

/// Stops a listener when its manager has no live tap reservations.
pub async fn shutdown_if_idle(manager: std::sync::Arc<dyn TapManager>) {
    let idle: Vec<std::sync::Arc<ServerHandle>> = {
        let mut registry = servers().lock().await;
        let keys: Vec<String> = registry
            .iter()
            .filter(|(_, handle)| {
                std::sync::Arc::ptr_eq(&handle.manager, &manager)
                    && handle.manager.active_count() == 0
            })
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| registry.remove(&key))
            .collect()
    };
    for handle in idle {
        stop_handle(handle).await;
    }
}

/// Stops every callback listener and the taps they serve.
pub async fn shutdown_all() {
    let handles: Vec<std::sync::Arc<ServerHandle>> = {
        let mut registry = servers().lock().await;
        std::mem::take(&mut *registry).into_values().collect()
    };
    let mut managers: Vec<std::sync::Arc<dyn TapManager>> = Vec::new();
    for handle in handles {
        if !managers
            .iter()
            .any(|manager| std::sync::Arc::ptr_eq(manager, &handle.manager))
        {
            managers.push(std::sync::Arc::clone(&handle.manager));
        }
        stop_handle(handle).await;
    }
    for manager in managers {
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, manager.shutdown()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "tap manager shutdown failed"),
            Err(_) => tracing::warn!("tap manager shutdown timed out"),
        }
    }
}

async fn stop_handle(handle: std::sync::Arc<ServerHandle>) {
    handle.cancel.cancel();
    if let Some(task) = handle.task.lock().await.take() {
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "tap callback task failed"),
            Err(_) => tracing::warn!("tap callback task did not stop before the timeout"),
        }
    }
}

async fn callback_handler(
    State(state): State<CallbackState>,
    Path((tap_id, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if headers.len() > MAX_HEADERS {
        return (
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            Json(json!({"error": "too many headers"})),
        )
            .into_response();
    }
    if body.len() > MAX_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": "callback body is too large"})),
        )
            .into_response();
    }

    let mut mapped = BTreeMap::new();
    for (name, value) in &headers {
        let Ok(value) = value.to_str() else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "header value is not valid UTF-8"})),
            )
                .into_response();
        };
        if value.len() > MAX_HEADER_VALUE {
            return (
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                Json(json!({"error": "header value is too large"})),
            )
                .into_response();
        }
        mapped.insert(name.as_str().to_ascii_lowercase(), value.to_owned());
    }

    let request = TapCallbackRequest {
        headers: mapped,
        body: body.to_vec(),
        received_at: Utc::now(),
    };
    match state.manager.deliver(&tap_id, &token, request).await {
        Ok(TapDelivery::Published(event)) => {
            state.hub.publish(SseEventKind::TapEvent { data: event });
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(TapDelivery::Filtered) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => callback_error_response(&error),
    }
}

fn callback_error_response(error: &CoreError) -> Response {
    let message = error.to_string();
    let status = if message.contains("unauthorized") {
        StatusCode::UNAUTHORIZED
    } else if message.contains("not found") {
        StatusCode::NOT_FOUND
    } else if message.contains("expired") {
        StatusCode::GONE
    } else if message.contains("invalid CloudEvent") {
        StatusCode::BAD_REQUEST
    } else if message.contains("not active") {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(json!({"error": message}))).into_response()
}
