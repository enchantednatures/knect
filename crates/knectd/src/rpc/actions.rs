//! `actions.*` RPC handlers. Task 11 replaces the bodies.

use knect_proto::RpcError;
use knect_proto::error::INTERNAL_ERROR;
use knect_proto::events::ActionResult;
use knect_proto::rpc::{FlaggerControlParams, SetTrafficSplitParams};

use crate::app::AppState;

pub(crate) async fn set_traffic_split(
    _state: &AppState,
    _params: SetTrafficSplitParams,
) -> Result<ActionResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "actions.set_traffic_split is implemented by Task 11",
    ))
}

pub(crate) async fn flagger_control(
    _state: &AppState,
    _params: FlaggerControlParams,
) -> Result<ActionResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "actions.flagger_control is implemented by Task 11",
    ))
}
