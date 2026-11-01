//! `knect tap` one-shot and follow modes.

use std::path::Path;

use clap::Args;
use futures::StreamExt;
use knect_proto::graph::NodeCategory;
use knect_proto::rpc::{TapStartParams, TapStopParams, methods};
use knect_proto::tap::{TapHandle, TapTarget, TapTargetKind};
use knect_proto::{SseEvent, SseEventKind};

use super::target;
use super::{CliError, json_text};

/// Arguments for `knect tap`.
#[derive(Debug, Clone, Args)]
pub struct TapArgs {
    /// Broker or Trigger target as `kind/name`.
    pub target: Option<String>,
    /// Follow events until interrupted.
    #[arg(long)]
    pub follow: bool,
    /// Namespace of the target.
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    /// List active taps.
    #[arg(long)]
    pub list: bool,
    /// Stop a tap by ID.
    #[arg(long, value_name = "ID")]
    pub stop: Option<String>,
    /// Emit JSON.
    #[arg(long)]
    pub json: bool,
    /// Maximum retained events.
    #[arg(long)]
    pub max_events: Option<u32>,
    /// Tap time-to-live in seconds.
    #[arg(long)]
    pub ttl: Option<u64>,
}

/// Runs the tap subcommand.
pub async fn run(
    args: TapArgs,
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<(), CliError> {
    let connected = target::connect(socket, daemon_url).await?;
    let client = std::sync::Arc::clone(&connected.client);
    if args.list {
        let value = client
            .call(methods::TAP_LIST.to_owned(), Some(serde_json::json!({})))
            .await?;
        let handles: Vec<TapHandle> = serde_json::from_value(value).map_err(|error| {
            CliError::Client(crate::client::ClientError::Protocol(format!(
                "cannot decode tap.list: {error}"
            )))
        })?;
        if args.json {
            println!("{}", json_text(&handles)?);
        } else if handles.is_empty() {
            println!("no active taps");
        } else {
            for handle in handles {
                print_handle(&handle);
            }
        }
        return Ok(());
    }
    if let Some(tap_id) = args.stop {
        return stop_tap(&client, &tap_id, args.json).await;
    }
    let Some(raw) = args.target.as_deref() else {
        return Err(CliError::Message(
            "tap requires <kind>/<name>, --list, or --stop <id>".to_owned(),
        ));
    };
    let parsed = target::parse_target(raw)?;
    let kind = match parsed.kind.as_ref().and_then(|kind| kind.category) {
        Some(NodeCategory::Broker) => TapTargetKind::Broker,
        Some(NodeCategory::Trigger) => TapTargetKind::Trigger,
        _ => {
            return Err(CliError::Message(
                "tap target must be a Broker or Trigger, for example broker/orders".to_owned(),
            ));
        }
    };
    let namespace = args
        .namespace
        .or(parsed.namespace)
        .unwrap_or_else(|| "default".to_owned());
    let params = TapStartParams {
        target: TapTarget {
            kind,
            namespace,
            name: parsed.name,
        },
        filter: None,
        max_events: args.max_events,
        ttl_secs: args.ttl,
    };
    let params = serde_json::to_value(params)
        .map_err(|error| CliError::Message(format!("cannot encode tap params: {error}")))?;
    let value = client
        .call(methods::TAP_START.to_owned(), Some(params))
        .await?;
    let handle: TapHandle = serde_json::from_value(value).map_err(|error| {
        CliError::Client(crate::client::ClientError::Protocol(format!(
            "cannot decode tap.start: {error}"
        )))
    })?;
    if args.json {
        println!("{}", json_text(&handle)?);
    } else {
        print_handle(&handle);
    }
    if !args.follow {
        return Ok(());
    }
    follow_events(&client, &handle.id, args.json).await?;
    stop_tap(&client, &handle.id, args.json).await
}

async fn stop_tap(
    client: &std::sync::Arc<dyn crate::client::RpcClient>,
    tap_id: &str,
    json: bool,
) -> Result<(), CliError> {
    let params = serde_json::to_value(TapStopParams {
        tap_id: tap_id.to_owned(),
    })
    .map_err(|error| CliError::Message(format!("cannot encode tap.stop: {error}")))?;
    let value = client
        .call(methods::TAP_STOP.to_owned(), Some(params))
        .await?;
    if json {
        println!("{}", json_text(&value)?);
    } else {
        println!("tap {tap_id} stopped");
    }
    Ok(())
}

async fn follow_events(
    client: &std::sync::Arc<dyn crate::client::RpcClient>,
    tap_id: &str,
    json: bool,
) -> Result<(), CliError> {
    let mut stream = client.subscribe(None, None).await?;
    loop {
        let next = tokio::select! {
            received = stream.next() => received,
            _ = tokio::signal::ctrl_c() => break,
        };
        match next {
            Some(Ok(event)) => print_tap_event(&event, tap_id, json),
            Some(Err(error)) => {
                eprintln!("tap {tap_id}: {error}");
                break;
            }
            None => break,
        }
    }
    Ok(())
}

fn print_tap_event(event: &SseEvent, tap_id: &str, json: bool) {
    let SseEventKind::TapEvent { data } = &event.kind else {
        return;
    };
    if json {
        if let Ok(text) = serde_json::to_string(data) {
            println!("{text}");
        }
        return;
    }
    println!(
        "{} {} {} {} tap={tap_id}",
        data.ce_id,
        data.ce_type,
        data.ce_source,
        data.ce_time
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "-".to_owned())
    );
}

fn print_handle(handle: &TapHandle) {
    println!(
        "{} {} {}/{} {:?} expires={} callback={}",
        handle.id,
        match handle.target.kind {
            TapTargetKind::Broker => "broker",
            TapTargetKind::Trigger => "trigger",
            TapTargetKind::Unknown => "unknown",
        },
        handle.target.namespace,
        handle.target.name,
        handle.state,
        handle.expires.to_rfc3339(),
        handle.callback_url
    );
}
