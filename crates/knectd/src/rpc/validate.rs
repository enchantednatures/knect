//! `policy.validate_manifest` RPC handler.

use knect_proto::RpcError;
use knect_proto::policy::ValidationResult;
use knect_proto::rpc::ValidateManifestParams;

use crate::app::AppState;

pub(crate) async fn validate_manifest(
    state: &AppState,
    params: ValidateManifestParams,
) -> Result<ValidationResult, RpcError> {
    state
        .actions
        .validate_manifest(params)
        .await
        .map_err(RpcError::from)
}
