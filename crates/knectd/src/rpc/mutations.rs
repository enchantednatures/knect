//! `mutations.*` RPC handlers. Task 9 replaces the body.

use knect_proto::RpcError;
use knect_proto::error::INTERNAL_ERROR;
use knect_proto::mutation::MutationDiff;
use knect_proto::rpc::GetDiffParams;

use crate::app::AppState;

pub(crate) async fn get_diff(
    _state: &AppState,
    _params: GetDiffParams,
) -> Result<MutationDiff, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "mutations.get_diff is implemented by Task 9",
    ))
}
