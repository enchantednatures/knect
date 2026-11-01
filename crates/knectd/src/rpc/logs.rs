//! `logs.*` RPC handlers. Task 12 replaces the bodies.

use knect_proto::RpcError;
use knect_proto::error::INTERNAL_ERROR;
use knect_proto::rpc::{LogsStopParams, LogsStopResult, LogsSubscribeParams, LogsSubscribeResult};

use crate::app::AppState;

pub(crate) async fn subscribe(
    _state: &AppState,
    _params: LogsSubscribeParams,
) -> Result<LogsSubscribeResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "logs.subscribe is implemented by Task 12",
    ))
}

pub(crate) async fn stop(
    _state: &AppState,
    _params: LogsStopParams,
) -> Result<LogsStopResult, RpcError> {
    Err(RpcError::new(
        INTERNAL_ERROR,
        "logs.stop is implemented by Task 12",
    ))
}
