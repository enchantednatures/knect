//! CloudEvent tap boundary.
//!
//! Task 12 replaces [`NoopTapManager`] with the ephemeral Trigger manager. The
//! foundation implementation never mutates a cluster.

use futures::future::BoxFuture;
use knect_proto::rpc::{TapStartParams, TapStopResult};
use knect_proto::tap::TapHandle;

use crate::error::CoreError;

/// Manages ephemeral CloudEvent taps.
pub trait TapManager: Send + Sync {
    /// Starts a tap and returns its handle.
    fn start(&self, params: TapStartParams) -> BoxFuture<'_, Result<TapHandle, CoreError>>;

    /// Stops a tap by ID.
    fn stop(&self, tap_id: String) -> BoxFuture<'_, Result<TapStopResult, CoreError>>;

    /// Lists active and recently expired taps.
    fn list(&self) -> BoxFuture<'_, Result<Vec<TapHandle>, CoreError>>;

    /// Reaps expired taps and returns their IDs.
    fn reap(&self) -> BoxFuture<'_, Result<Vec<String>, CoreError>>;
}

/// Tap manager used until Task 12 installs the real implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTapManager;

impl TapManager for NoopTapManager {
    fn start(&self, _params: TapStartParams) -> BoxFuture<'_, Result<TapHandle, CoreError>> {
        Box::pin(async move {
            Err(CoreError::Unsupported(
                "tap.start is implemented by Task 12".to_owned(),
            ))
        })
    }

    fn stop(&self, _tap_id: String) -> BoxFuture<'_, Result<TapStopResult, CoreError>> {
        Box::pin(async move { Ok(TapStopResult { removed: false }) })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<TapHandle>, CoreError>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn reap(&self) -> BoxFuture<'_, Result<Vec<String>, CoreError>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::tap::{TapTarget, TapTargetKind};

    #[tokio::test]
    async fn noop_tap_manager_is_safe() {
        let manager = NoopTapManager;
        assert!(manager.list().await.expect("list").is_empty());
        assert!(manager.reap().await.expect("reap").is_empty());
        assert!(
            !manager
                .stop("missing".to_owned())
                .await
                .expect("stop")
                .removed
        );

        let error = manager
            .start(TapStartParams {
                target: TapTarget {
                    kind: TapTargetKind::Broker,
                    namespace: "default".to_owned(),
                    name: "orders".to_owned(),
                },
                filter: None,
                max_events: None,
                ttl_secs: None,
            })
            .await
            .expect_err("start unsupported");
        assert_eq!(error.code(), knect_proto::error::UNSUPPORTED_GVK);
    }

    #[test]
    fn tap_trait_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn TapManager>();
        let _manager: std::sync::Arc<dyn TapManager> = std::sync::Arc::new(NoopTapManager);
    }
}
