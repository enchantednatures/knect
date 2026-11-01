//! `policy.validate_manifest` RPC handler. Task 11 replaces the body.

use knect_proto::RpcError;
use knect_proto::error::INTERNAL_ERROR;
use knect_proto::policy::ValidationResult;
use knect_proto::rpc::ValidateManifestParams;

use crate::app::AppState;

pub(crate) async fn validate_manifest(
    _state: &AppState,
    _params: ValidateManifestParams,
) -> Result<ValidationResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "policy.validate_manifest is implemented by Task 11",
    ))
}
