//! `logs.*` RPC handlers and pod log subscriptions.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use knect_core::ClusterReader;
use knect_core::kube::LogOptions;
use knect_proto::Gvk;
use knect_proto::RpcError;
use knect_proto::error::{INVALID_PARAMS, UNSUPPORTED_GVK};
use knect_proto::events::SseEventKind;
use knect_proto::rpc::{LogsStopParams, LogsStopResult, LogsSubscribeParams, LogsSubscribeResult};
use knect_proto::tap::{LogLine, LogStream, PodRef};
use regex::Regex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::AppState;

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POD_GROUP: &str = "";

static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

struct Subscription {
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

fn subscriptions() -> &'static Mutex<BTreeMap<String, Subscription>> {
    static SUBSCRIPTIONS: OnceLock<Mutex<BTreeMap<String, Subscription>>> = OnceLock::new();
    SUBSCRIPTIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn lock_subscriptions() -> MutexGuard<'static, BTreeMap<String, Subscription>> {
    match subscriptions().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) async fn subscribe(
    state: &AppState,
    params: LogsSubscribeParams,
) -> Result<LogsSubscribeResult, RpcError> {
    let reader = state.reader.clone().ok_or_else(|| {
        RpcError::new(
            UNSUPPORTED_GVK,
            "logs.subscribe requires a Kubernetes reader",
        )
    })?;
    if params.pods.is_empty() {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "logs.subscribe requires at least one pod",
        ));
    }
    for pod in &params.pods {
        if pod.namespace.trim().is_empty() || pod.name.trim().is_empty() {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "pod namespace and name are required",
            ));
        }
    }
    let filter = match params.filter.as_deref() {
        Some(pattern) => Some(Regex::new(pattern).map_err(|error| {
            RpcError::new(
                INVALID_PARAMS,
                format!("invalid logs.subscribe filter: {error}"),
            )
        })?),
        None => None,
    };

    let subscription_id = format!(
        "logs-{}-{}",
        std::process::id(),
        SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let cancel = CancellationToken::new();
    let mut tasks = Vec::with_capacity(params.pods.len());
    for pod in params.pods {
        let reader = Arc::clone(&reader);
        let cancel = cancel.child_token();
        let hub = Arc::clone(&state.events);
        let filter = filter.clone();
        let tail_lines = params.tail_lines;
        tasks.push(tokio::spawn(async move {
            stream_pod(reader, pod, tail_lines, filter, hub, cancel).await;
        }));
    }

    lock_subscriptions().insert(subscription_id.clone(), Subscription { cancel, tasks });
    Ok(LogsSubscribeResult { subscription_id })
}

async fn stream_pod(
    reader: Arc<dyn ClusterReader>,
    pod: PodRef,
    tail_lines: Option<u32>,
    filter: Option<Regex>,
    hub: Arc<crate::transport::sse::EventHub>,
    cancel: CancellationToken,
) {
    let container = resolve_container(&reader, &pod).await;
    let options = LogOptions {
        container: Some(container.clone()),
        tail_lines,
        timestamps: true,
        follow: true,
    };
    let mut stream = reader.log_stream(&pod.namespace, &pod.name, options);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            item = stream.next() => {
                match item {
                    Some(Ok(raw)) => {
                        let line = parse_line(&pod, &container, &raw);
                        let matches = filter
                            .as_ref()
                            .is_none_or(|filter| filter.is_match(&line.message));
                        if matches {
                            hub.publish(SseEventKind::LogLine { data: line });
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(
                            namespace = %pod.namespace,
                            pod = %pod.name,
                            %error,
                            "pod log stream ended with an error"
                        );
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

async fn resolve_container(reader: &Arc<dyn ClusterReader>, pod: &PodRef) -> String {
    if let Some(container) = &pod.container {
        return container.clone();
    }
    let gvk = Gvk::new(POD_GROUP, "v1", "Pod");
    let Ok(object) = reader
        .get(&gvk, Some(pod.namespace.as_str()), pod.name.as_str())
        .await
    else {
        return "default".to_owned();
    };
    object
        .metadata
        .annotations
        .as_ref()
        .and_then(|annotations| {
            annotations
                .get("kubectl.kubernetes.io/default-container")
                .cloned()
        })
        .or_else(|| {
            object
                .data
                .get("spec")
                .and_then(|spec| spec.get("containers"))
                .and_then(serde_json::Value::as_array)
                .and_then(|containers| containers.first())
                .and_then(|container| container.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "default".to_owned())
}

fn parse_line(pod: &PodRef, container: &str, raw: &str) -> LogLine {
    let (timestamp, message) = split_timestamp(raw);
    LogLine {
        namespace: pod.namespace.clone(),
        pod: pod.name.clone(),
        container: container.to_owned(),
        timestamp,
        message: crate::logging::redact_message(&message),
        stream: LogStream::Stdout,
    }
}

fn split_timestamp(raw: &str) -> (DateTime<Utc>, String) {
    if let Some((prefix, rest)) = raw.split_once(' ')
        && let Ok(timestamp) = DateTime::parse_from_rfc3339(prefix)
    {
        return (timestamp.with_timezone(&Utc), rest.to_owned());
    }
    (Utc::now(), raw.to_owned())
}

pub(crate) async fn stop(
    _state: &AppState,
    params: LogsStopParams,
) -> Result<LogsStopResult, RpcError> {
    let subscription = lock_subscriptions().remove(&params.subscription_id);
    let Some(subscription) = subscription else {
        return Ok(LogsStopResult { stopped: false });
    };
    subscription.cancel.cancel();
    for task in subscription.tasks {
        let mut task = task;
        match tokio::time::timeout(STOP_TIMEOUT, &mut task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "log subscription task failed");
            }
            Err(_) => {
                task.abort();
                let _ = task.await;
            }
        }
    }
    Ok(LogsStopResult { stopped: true })
}
