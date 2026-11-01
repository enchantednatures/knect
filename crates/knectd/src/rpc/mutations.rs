//! `mutations.*` RPC handlers.

use knect_core::mutations::{MutationError, diff};
use knect_proto::RpcError;
use knect_proto::error::{INTERNAL_ERROR, NOT_FOUND};
use knect_proto::mutation::MutationDiff;
use knect_proto::rpc::GetDiffParams;

use crate::app::AppState;

/// Returns the three-way mutation attribution for one stored resource.
pub(crate) async fn get_diff(
    state: &AppState,
    params: GetDiffParams,
) -> Result<MutationDiff, RpcError> {
    let object = {
        let store = state.store.read().await;
        store.get(&params.id)
    };
    let Some(object) = object else {
        return Err(RpcError::new(
            NOT_FOUND,
            format!("resource {} is not present in the store", params.id),
        ));
    };
    if object
        .metadata
        .managed_fields
        .as_ref()
        .is_none_or(Vec::is_empty)
    {
        return Err(RpcError::new(
            NOT_FOUND,
            format!("resource {} has no managedFields", params.id),
        ));
    }

    diff(&object, params.mode.unwrap_or_default()).map_err(rpc_error)
}

fn rpc_error(error: MutationError) -> RpcError {
    let message = error.to_string();
    match error {
        MutationError::MissingManagedFields => RpcError::new(NOT_FOUND, message),
        _ => RpcError::new(INTERNAL_ERROR, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_core::config::Config;
    use knect_proto::ResourceId;
    use knect_proto::mutation::{MutationDiffMode, MutationSource};
    use kube::api::DynamicObject;

    fn object(fixture: &str) -> DynamicObject {
        let raw = std::fs::read_to_string(format!(
            "{}/../../testdata/fixtures/mutations/{fixture}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture is readable");
        serde_json::from_str(&raw).expect("fixture is a valid object")
    }

    fn id() -> ResourceId {
        ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            "knect-e2e",
            "order-processor-mutated",
        )
    }

    #[tokio::test]
    async fn mutations_handler_returns_attributed_diff() {
        let state = AppState::new(Config::default(), true);
        let object = object("kservice-mutated.json");
        state.store.write().await.upsert(object);

        let diff = get_diff(
            &state,
            GetDiffParams {
                id: id(),
                mode: Some(MutationDiffMode::ThreeWay),
            },
        )
        .await
        .expect("diff is produced");

        assert_eq!(diff.resource, id());
        assert!(diff.user_intent.is_some());
        assert!(diff.knative_defaults.is_some());
        assert!(diff.policy_mutators.is_some());
        assert!(diff.lines.iter().any(|line| {
            line.owner.as_deref() == Some("kyverno")
                && line.side == MutationSource::PolicyMutators
                && line.text.contains("knect.dev/managed")
        }));
        assert!(diff.lines.iter().any(|line| {
            line.owner.as_deref() == Some("webhook.serving.knative.dev")
                && line.side == MutationSource::KnativeDefaults
        }));
    }

    #[tokio::test]
    async fn mutations_handler_reports_missing_objects() {
        let state = AppState::new(Config::default(), true);
        let error = get_diff(
            &state,
            GetDiffParams {
                id: id(),
                mode: None,
            },
        )
        .await
        .expect_err("missing object");
        assert_eq!(error.code, NOT_FOUND);
    }

    #[tokio::test]
    async fn mutations_handler_requires_managed_fields() {
        let state = AppState::new(Config::default(), true);
        let mut object = object("kservice-order-processor.json");
        object.metadata.managed_fields = None;
        object.metadata.name = Some("order-processor".to_owned());
        state.store.write().await.upsert(object);

        let error = get_diff(
            &state,
            GetDiffParams {
                id: ResourceId::namespaced(
                    "serving.knative.dev/v1",
                    "Service",
                    "knect-e2e",
                    "order-processor",
                ),
                mode: None,
            },
        )
        .await
        .expect_err("managedFields required");
        assert_eq!(error.code, NOT_FOUND);
        assert!(error.message.contains("managedFields"));
    }

    #[tokio::test]
    async fn mutations_handler_never_renders_secret_values() {
        let state = AppState::new(Config::default(), true);
        let secret: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "credentials",
                "namespace": "knect-e2e",
                "managedFields": [{
                    "apiVersion": "v1",
                    "fieldsType": "FieldsV1",
                    "fieldsV1": {"f:data": {"f:password": {}}},
                    "manager": "kubectl-client-side-apply",
                    "operation": "Update"
                }]
            },
            "data": {"password": "hunter2-knect-sentinel"}
        }))
        .expect("secret fixture");
        let id = ResourceId::namespaced("v1", "Secret", "knect-e2e", "credentials");
        state.store.write().await.upsert(secret);

        let diff = get_diff(
            &state,
            GetDiffParams {
                id,
                mode: Some(MutationDiffMode::Managers),
            },
        )
        .await
        .expect("secret diff");
        let rendered = format!(
            "{}{}{}{}",
            diff.user_intent.unwrap_or_default(),
            diff.knative_defaults.unwrap_or_default(),
            diff.policy_mutators.unwrap_or_default(),
            diff.lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>()
        );
        assert!(!rendered.contains("hunter2-knect-sentinel"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
