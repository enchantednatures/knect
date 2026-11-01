//! `tap.*` RPC handlers. Task 12 replaces the bodies.

use knect_proto::RpcError;
use knect_proto::error::INTERNAL_ERROR;
use knect_proto::rpc::{TapStartParams, TapStopParams, TapStopResult};
use knect_proto::tap::TapHandle;

use crate::app::AppState;

pub(crate) async fn start(
    _state: &AppState,
    _params: TapStartParams,
) -> Result<TapHandle, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "tap.start is implemented by Task 12",
    ))
}

pub(crate) async fn stop(
    _state: &AppState,
    _params: TapStopParams,
) -> Result<TapStopResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "tap.stop is implemented by Task 12",
    ))
}

pub(crate) async fn list(_state: &AppState) -> Result<Vec<TapHandle>, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "tap.list is implemented by Task 12",
    ))
}
