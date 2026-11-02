//! `knectd` entrypoint.
//!
//! Parses flags, loads configuration, initializes logging, starts the runtime,
//! HTTP listener, and UDS listener, then waits for SIGINT/SIGTERM and shuts
//! down gracefully. Fixture mode never constructs a Kubernetes client.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use clap::Parser;
use knect_core::ClusterReader;
use knect_core::actions::KubeActions;
use knect_core::config::{Config, ConfigOverrides};
use knect_core::kube::KubeReader;
use knect_core::tap::{TapManagerConfig, TriggerTapManager};
use knect_core::{NoopCache, RedbCache, SnapshotCache, snapshot_limit_bytes};
use knectd::app::AppState;
use knectd::error::DaemonError;
use knectd::fixture::FixtureMode;
use knectd::runtime::Runtime;
use knectd::transport::{http, uds};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Parser)]
#[command(
    name = "knectd",
    version = knectd::VERSION,
    about = "Knative topology daemon"
)]
struct Args {
    /// Configuration file. Defaults to ~/.config/knect/config.toml when present.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Kubernetes context override.
    #[arg(long)]
    context: Option<String>,
    /// Kubeconfig path override. Ignored in fixture mode.
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
    /// HTTP listen address override.
    #[arg(long)]
    http_addr: Option<String>,
    /// Unix domain socket path override.
    #[arg(long)]
    socket_path: Option<PathBuf>,
    /// Cache directory override.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Fixture mode. With no DIR, start an empty fixture without Kubernetes.
    #[arg(long, value_name = "DIR", num_args = 0..=1)]
    fixture: Option<Option<PathBuf>>,
    /// Enable guarded write actions.
    #[arg(long)]
    allow_actions: bool,
    /// Tracing filter override.
    #[arg(long)]
    log: Option<String>,
    /// Log format override (`human` or `json`).
    #[arg(long)]
    log_format: Option<String>,
}

impl Args {
    fn overrides(&self) -> ConfigOverrides {
        ConfigOverrides {
            context: self.context.clone(),
            kubeconfig: self.kubeconfig.clone(),
            http_addr: self.http_addr.clone(),
            socket_path: self.socket_path.clone(),
            cache_dir: self.cache_dir.clone(),
            allow_actions: self.allow_actions.then_some(true),
            log: self.log.clone(),
            log_format: self.log_format.clone(),
        }
    }

    fn fixture_mode(&self) -> FixtureMode {
        match self.fixture.clone() {
            None => FixtureMode::Disabled,
            Some(path) => FixtureMode::Enabled(path),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = Args::parse();
    let config = Config::load(args.config.as_deref(), args.overrides())
        .context("cannot load configuration")?;
    knectd::logging::init(&config).context("cannot initialize logging")?;

    let fixture = args.fixture_mode();
    let cache: Arc<dyn SnapshotCache> = if config.cache.enabled && !fixture.is_enabled() {
        let cache_dir = PathBuf::from(&config.daemon.cache_dir);
        let limit = snapshot_limit_bytes(config.cache.max_snapshot_bytes);
        match RedbCache::open(&cache_dir, limit) {
            Ok(cache) => {
                tracing::info!(path = %cache_dir.display(), "snapshot cache opened");
                Arc::new(cache)
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    path = %cache_dir.display(),
                    "snapshot cache unavailable; continuing without persistence"
                );
                Arc::new(NoopCache)
            }
        }
    } else {
        Arc::new(NoopCache)
    };
    let cancel = CancellationToken::new();
    let mut state = AppState::new(config.clone(), fixture.is_enabled()).with_cache(cache);
    if !fixture.is_enabled() {
        let context = config.daemon.context.as_str();
        match KubeReader::from_context(
            (!context.is_empty()).then_some(context),
            args.kubeconfig.as_deref(),
        )
        .await
        {
            Ok(reader) => {
                let reader: Arc<dyn ClusterReader> = Arc::new(reader);
                state = state.with_cluster_reader(Arc::clone(&reader));
                state = state.with_actions(Arc::new(KubeActions::new(Arc::clone(&reader))));
                let tap = Arc::new(TriggerTapManager::new(
                    Arc::clone(&reader),
                    TapManagerConfig::from(&config.tap),
                ));
                state = state.with_tap(tap.clone());
                match tokio::time::timeout(Duration::from_secs(10), tap.gc_startup()).await {
                    Ok(Ok(removed)) if !removed.is_empty() => {
                        tracing::info!(removed = removed.len(), "removed orphaned tap resources");
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "tap startup GC failed; will retry on first tap.start");
                    }
                    Err(_) => tracing::warn!("tap startup GC timed out"),
                }
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "cannot construct the Kubernetes reader; live tap and log features are disabled"
                );
            }
        }
    }
    let socket_path = PathBuf::from(&config.daemon.socket_path);
    let http_addr = config.daemon.http_addr.clone();

    tracing::info!(
        version = knectd::VERSION,
        context = %state.context(),
        fixture = fixture.is_enabled(),
        %http_addr,
        socket = %socket_path.display(),
        "starting knectd"
    );

    let runtime = Runtime::new(state.clone(), fixture.clone(), cancel.clone())
        .with_kubeconfig(args.kubeconfig.clone());
    runtime.start().await.context("runtime startup failed")?;

    let mut tasks: JoinSet<Result<(), DaemonError>> = JoinSet::new();
    tasks.spawn(runtime.run());
    tasks.spawn(http::serve(
        state.clone(),
        http_addr.clone(),
        cancel.clone(),
    ));
    tasks.spawn(uds::serve(state, socket_path.clone(), cancel.clone()));

    let mut failure: Option<anyhow::Error> = None;
    tokio::select! {
        signal = shutdown_signal() => {
            match signal {
                Ok(()) => tracing::info!("shutdown signal received"),
                Err(error) => failure = Some(anyhow!("signal handler failed: {error}")),
            }
        }
        joined = tasks.join_next() => {
            match joined {
                Some(Ok(Ok(()))) => tracing::info!("supervised task exited"),
                Some(Ok(Err(error))) => failure = Some(anyhow!(error)),
                Some(Err(error)) => failure = Some(anyhow!("task join failed: {error}")),
                None => failure = Some(anyhow!("all supervised tasks exited")),
            }
        }
    }

    cancel.cancel();
    knectd::tap_callback::shutdown_all().await;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "task exited with error during shutdown"),
            Err(error) => tracing::warn!(%error, "task join failed during shutdown"),
        }
    }
    uds::remove_socket(&socket_path);
    tracing::info!("knectd stopped");

    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
