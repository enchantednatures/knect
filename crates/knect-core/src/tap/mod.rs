//! CloudEvent tap boundary.
//!
//! [`TriggerTapManager`] creates an ephemeral Knative Trigger per tap, receives
//! CloudEvents on a token-guarded callback, and retains a bounded ring of
//! events for the daemon's SSE hub. The callback HTTP listener lives in
//! `knectd`; this module owns the security-sensitive delivery and lifecycle
//! logic so it is testable without a cluster.

mod callback;
mod manager;
mod token;
mod trigger;

use std::net::SocketAddr;

use futures::future::BoxFuture;
use knect_proto::rpc::{TapStartParams, TapStopResult};
use knect_proto::tap::TapHandle;

use crate::error::CoreError;

pub use callback::{TapCallbackRequest, TapDelivery, redact_headers};
pub use manager::{TapManagerConfig, TriggerTapManager};

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

    /// Delivers one token-authenticated callback request to a tap.
    ///
    /// The default rejects the delivery so existing test doubles keep compiling
    /// without claiming callback support.
    fn deliver(
        &self,
        _tap_id: &str,
        _token: &str,
        _request: TapCallbackRequest,
    ) -> BoxFuture<'_, Result<TapDelivery, CoreError>> {
        Box::pin(async move {
            Err(CoreError::Unsupported(
                "tap delivery is not supported by this manager".to_owned(),
            ))
        })
    }

    /// Records the address the daemon callback listener actually bound.
    fn set_callback_addr(&self, _addr: SocketAddr) {}

    /// Returns the number of live tap reservations.
    fn active_count(&self) -> usize {
        0
    }

    /// Stops every tap and releases manager-owned background work.
    fn shutdown(&self) -> BoxFuture<'_, Result<(), CoreError>> {
        Box::pin(async move { Ok(()) })
    }
}

/// Tap manager used until a real implementation is installed.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTapManager;

impl TapManager for NoopTapManager {
    fn start(&self, _params: TapStartParams) -> BoxFuture<'_, Result<TapHandle, CoreError>> {
        Box::pin(async move {
            Err(CoreError::Unsupported(
                "tap.start requires a Kubernetes reader".to_owned(),
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
        assert_eq!(manager.active_count(), 0);
        manager.shutdown().await.expect("shutdown");

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
