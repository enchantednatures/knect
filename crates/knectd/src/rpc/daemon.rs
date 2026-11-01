//! `daemon.*`-adjacent handlers that read live runtime state.

use knect_proto::rpc::{DaemonStatus, RefreshResult};

use crate::VERSION;
use crate::app::AppState;

/// Reports real runtime state: generation, watcher health, uptime, cache, and
/// tap count. No value is synthesized from configuration except the cache
/// enable flag, which is exactly the configured gate.
pub(crate) async fn status(state: &AppState) -> DaemonStatus {
    let tap_count = state
        .tap
        .list()
        .await
        .map_or(0, |taps| u32::try_from(taps.len()).unwrap_or(u32::MAX));
    DaemonStatus {
        version: VERSION.to_owned(),
        schema_version: knect_proto::SCHEMA_VERSION,
        context: state.context(),
        generation: state.generation(),
        watchers: state.watchers(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        actions_enabled: state.actions_enabled(),
        tap_count,
        started_at: Some(state.started_at_utc),
        cache_enabled: state.config.cache.enabled,
        last_resync: state.last_resync(),
    }
}

/// Signals the runtime to rediscover. The runtime owns the rebuild; this
/// handler never fakes a topology change.
pub(crate) fn refresh(state: &AppState) -> RefreshResult {
    state.refresh.notify_one();
    RefreshResult {
        generation: state.generation(),
    }
}
