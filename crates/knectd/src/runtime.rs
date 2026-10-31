//! Runtime supervisor skeleton.
//!
//! Normal mode does not contact Kubernetes in this milestone: it seeds an empty
//! topology and stays quiet until Task 3 installs discovery/watchers and Task 5
//! installs the recompute loop. Fixture mode loads the fixture store and serves
//! an empty topology without any cluster access.

use knect_core::{CoreError, SnapshotCache};
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::app::empty_topology;
use crate::error::DaemonError;
use crate::fixture::{self, FixtureMode};

/// One daemon runtime instance.
pub struct Runtime {
    state: AppState,
    fixture: FixtureMode,
    cancel: CancellationToken,
}

impl Runtime {
    /// Creates a runtime. `start` must run before `run`.
    #[must_use]
    pub fn new(state: AppState, fixture: FixtureMode, cancel: CancellationToken) -> Self {
        Self {
            state,
            fixture,
            cancel,
        }
    }

    /// Performs startup work and installs the initial topology.
    pub async fn start(&self) -> Result<(), DaemonError> {
        if self.fixture.is_enabled() {
            let outcome = fixture::initialize(&self.state, &self.fixture).await?;
            tracing::info!(
                context = %outcome.context,
                objects = outcome.object_count,
                "fixture mode started; no Kubernetes client was constructed"
            );
            return Ok(());
        }
        let mut loaded = false;
        if self.state.config.cache.enabled {
            match self.state.cache.load(&self.state.context()) {
                Ok(Some(topology)) => {
                    tracing::info!(
                        generation = topology.generation,
                        "loaded cached topology snapshot"
                    );
                    self.state.set_topology(topology);
                    loaded = true;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "snapshot cache load failed; starting empty");
                }
            }
        }
        if !loaded {
            let generation = self.state.generation();
            self.state
                .set_topology(empty_topology(&self.state.context(), generation));
        }
        Ok(())
    }

    /// Waits for shutdown and flushes the cache.
    pub async fn run(self) -> Result<(), DaemonError> {
        self.cancel.cancelled().await;
        if self.state.config.cache.enabled {
            self.flush_cache()?;
        }
        tracing::info!("runtime stopped");
        Ok(())
    }

    fn flush_cache(&self) -> Result<(), DaemonError> {
        self.state.cache.flush().map_err(|error| match error {
            CoreError::Cache(message) => {
                DaemonError::Runtime(format!("cache flush failed: {message}"))
            }
            other => DaemonError::Runtime(other.to_string()),
        })
    }
}

/// Compile-time proof that the cache trait is object safe from the daemon side.
#[must_use]
pub fn cache_as_dyn(cache: std::sync::Arc<dyn SnapshotCache>) -> std::sync::Arc<dyn SnapshotCache> {
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_core::config::Config;

    #[tokio::test]
    async fn fixture_runtime_installs_empty_fixture_topology() {
        let state = AppState::new(Config::default(), true);
        let cancel = CancellationToken::new();
        let runtime = Runtime::new(state.clone(), FixtureMode::Enabled(None), cancel.clone());
        runtime.start().await.expect("start");
        assert_eq!(state.topology().context, "fixture");
        assert_eq!(state.topology().generation, 0);
        assert!(state.topology().nodes.is_empty());
        cancel.cancel();
        runtime.run().await.expect("run");
    }

    #[tokio::test]
    async fn normal_runtime_does_not_fake_a_cluster() {
        let mut config = Config::default();
        config.daemon.context = "not-a-cluster".to_owned();
        let state = AppState::new(config, false);
        let cancel = CancellationToken::new();
        let runtime = Runtime::new(state.clone(), FixtureMode::Disabled, cancel.clone());
        runtime.start().await.expect("start");
        assert_eq!(state.topology().context, "not-a-cluster");
        assert!(state.topology().nodes.is_empty());
        cancel.cancel();
        runtime.run().await.expect("run");
    }
}
