//! Validated executor backed by a [`ClusterReader`].

use std::sync::Arc;

use chrono::Utc;
use futures::future::BoxFuture;
use knect_proto::ResourceId;
use knect_proto::events::ActionResult;
use knect_proto::policy::ValidationResult;
use knect_proto::rpc::{FlaggerControlParams, SetTrafficSplitParams, ValidateManifestParams};

use crate::error::CoreError;
use crate::kube::ClusterReader;

use super::{ActionExecutor, flagger, traffic, validate};

/// Guarded executor that validates requests and writes through a reader.
#[derive(Clone)]
pub struct KubeActions {
    reader: Arc<dyn ClusterReader>,
}

impl KubeActions {
    /// Creates an executor that performs every read and write through `reader`.
    #[must_use]
    pub fn new(reader: Arc<dyn ClusterReader>) -> Self {
        Self { reader }
    }
}

impl ActionExecutor for KubeActions {
    fn set_traffic_split(
        &self,
        params: SetTrafficSplitParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>> {
        Box::pin(async move { traffic::set_traffic_split(self.reader.as_ref(), params).await })
    }

    fn flagger_control(
        &self,
        params: FlaggerControlParams,
    ) -> BoxFuture<'_, Result<ActionResult, CoreError>> {
        Box::pin(async move { flagger::flagger_control(self.reader.as_ref(), params).await })
    }

    fn validate_manifest(
        &self,
        params: ValidateManifestParams,
    ) -> BoxFuture<'_, Result<ValidationResult, CoreError>> {
        Box::pin(async move { validate::validate_manifest(self.reader.as_ref(), params).await })
    }
}

pub(super) fn action_result(
    action: &str,
    message: String,
    dry_run: bool,
    resource: Option<ResourceId>,
) -> ActionResult {
    ActionResult {
        action: action.to_owned(),
        success: true,
        message,
        dry_run,
        resource,
        generation: None,
        warnings: Vec::new(),
        ts: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kube::FakeReader;

    #[test]
    fn executor_accepts_a_dyn_reader() {
        let reader: Arc<dyn ClusterReader> = Arc::new(FakeReader::new());
        let actions = KubeActions::new(reader);
        let _: &dyn ActionExecutor = &actions;
    }

    #[test]
    fn action_result_marks_success_and_resource() {
        let id = ResourceId::namespaced("serving.knative.dev/v1", "Service", "default", "hello");
        let result = action_result("actions.test", "done".to_owned(), true, Some(id.clone()));
        assert!(result.success);
        assert!(result.dry_run);
        assert_eq!(result.resource, Some(id));
        assert_eq!(result.action, "actions.test");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn executor_is_constructible_as_a_function() {
        let _: fn(Arc<dyn ClusterReader>) -> KubeActions = KubeActions::new;
    }
}
