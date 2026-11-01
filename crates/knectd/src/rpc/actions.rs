//! `actions.*` RPC handlers.

use knect_proto::RpcError;
use knect_proto::events::ActionResult;
use knect_proto::rpc::{FlaggerControlParams, SetTrafficSplitParams};

use crate::app::AppState;

pub(crate) async fn set_traffic_split(
    state: &AppState,
    params: SetTrafficSplitParams,
) -> Result<ActionResult, RpcError> {
    state
        .actions
        .set_traffic_split(params)
        .await
        .map_err(RpcError::from)
}

pub(crate) async fn flagger_control(
    state: &AppState,
    params: FlaggerControlParams,
) -> Result<ActionResult, RpcError> {
    state
        .actions
        .flagger_control(params)
        .await
        .map_err(RpcError::from)
}
