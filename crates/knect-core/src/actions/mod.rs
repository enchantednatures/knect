//! Guarded write-action boundary.
//!
//! [`KubeActions`] performs every mutation through the [`ClusterReader`]
//! boundary and validates the request before a request reaches the API server.
//! [`DisabledActions`] remains the fail-closed fallback installed when the
//! daemon has no action driver, so a misconfigured daemon cannot write.
//!
//! [`ClusterReader`]: crate::kube::ClusterReader

mod executor;
mod flagger;
mod traffic;
mod validate;

use futures::future::BoxFuture;
use knect_proto::events::ActionResult;
use knect_proto::policy::ValidationResult;
use knect_proto::rpc::{FlaggerControlParams, SetTrafficSplitParams, ValidateManifestParams};

pub use executor::KubeActions;

use crate::error::CoreError;

/// Executes guarded, validated cluster writes.
pub trait ActionExecutor: Send + Sync {
    /// Changes a Knative service traffic split.
    fn set_traffic_split(
        &self,
        params: SetTrafficSplitParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>>;

    /// Promotes or aborts a Flagger canary.
    fn flagger_control(
        &self,
        params: FlaggerControlParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>>;

    /// Validates a manifest with a server-side dry run.
    fn validate_manifest(
        &self,
        params: ValidateManifestParams,
    ) -> BoxFuture<'_, Result<ValidationResult, CoreError>>;
}

/// Executor used when actions are disabled or no driver is installed.
#[derive(Debug, Default, Clone, Copy)]
pub struct DisabledActions;

impl DisabledActions {
    fn disabled(message: impl Into<String>) -> CoreError {
        CoreError::Action(format!(
            "{}; start knectd with --allow-actions and enable actions in config",
            message.into()
        ))
    }
}

impl ActionExecutor for DisabledActions {
    fn set_traffic_split(
        &self,
        _params: SetTrafficSplitParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>> {
        Box::pin(async move { Err(Self::disabled("set_traffic_split is disabled")) })
    }

    fn flagger_control(
        &self,
        _params: FlaggerControlParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>> {
        Box::pin(async move { Err(Self::disabled("flagger_control is disabled")) })
    }

    fn validate_manifest(
        &self,
        _params: ValidateManifestParams,
    ) -> BoxFuture<'_, Result<ValidationResult, CoreError>> {
        Box::pin(async move { Err(Self::disabled("validate_manifest is disabled")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::rpc::FlaggerAction;

    #[tokio::test]
    async fn disabled_actions_fail_closed() {
        let actions = DisabledActions;
        let error = actions
            .set_traffic_split(SetTrafficSplitParams {
                namespace: "default".to_owned(),
                service: "hello".to_owned(),
                splits: Vec::new(),
                dry_run: Some(true),
            })
            .await
            .expect_err("disabled");
        assert_eq!(error.code(), knect_proto::error::ACTIONS_DISABLED);

        let error = actions
            .flagger_control(FlaggerControlParams {
                namespace: "default".to_owned(),
                canary: "hello".to_owned(),
                action: FlaggerAction::Promote,
                dry_run: None,
            })
            .await
            .expect_err("disabled");
        assert_eq!(error.code(), knect_proto::error::ACTIONS_DISABLED);

        let error = actions
            .validate_manifest(ValidateManifestParams {
                manifest: String::new(),
                namespace: None,
                default_kind: None,
            })
            .await
            .expect_err("disabled");
        assert_eq!(error.code(), knect_proto::error::ACTIONS_DISABLED);
    }

    #[test]
    fn action_trait_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ActionExecutor>();
        let _executor: std::sync::Arc<dyn ActionExecutor> = std::sync::Arc::new(DisabledActions);
    }
}
