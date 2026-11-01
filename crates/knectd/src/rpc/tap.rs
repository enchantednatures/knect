//! `tap.*` RPC handlers.

use std::sync::Arc;

use knect_core::CoreError;
use knect_proto::RpcError;
use knect_proto::error::{INTERNAL_ERROR, INVALID_PARAMS, NOT_FOUND, TAP_FORBIDDEN, TAP_LIMIT};
use knect_proto::rpc::{TapStartParams, TapStopParams, TapStopResult};
use knect_proto::tap::{TapHandle, TapTargetKind};

use crate::app::AppState;

pub(crate) async fn start(state: &AppState, params: TapStartParams) -> Result<TapHandle, RpcError> {
    if state.fixture {
        return Err(RpcError::new(
            TAP_FORBIDDEN,
            "tap.start is unavailable in fixture mode",
        ));
    }
    if !state.config.tap.enabled {
        return Err(RpcError::new(
            TAP_FORBIDDEN,
            "tap is disabled by configuration",
        ));
    }
    if params.target.kind == TapTargetKind::Unknown {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "tap target kind must be 'broker' or 'trigger'",
        ));
    }
    if state.reader.is_none() {
        return Err(RpcError::new(
            TAP_FORBIDDEN,
            "tap.start requires a Kubernetes reader",
        ));
    }

    crate::tap_callback::ensure_for(
        &state.config.tap,
        Arc::clone(&state.tap),
        Arc::clone(&state.events),
    )
    .await
    .map_err(|error| {
        RpcError::new(
            INTERNAL_ERROR,
            format!("cannot start the tap callback listener: {error}"),
        )
    })?;

    let outcome = state.tap.start(params).await;
    if outcome.is_err() {
        crate::tap_callback::shutdown_if_idle(Arc::clone(&state.tap)).await;
    }
    outcome.map_err(map_core_error)
}

pub(crate) async fn stop(
    state: &AppState,
    params: TapStopParams,
) -> Result<TapStopResult, RpcError> {
    let outcome = state.tap.stop(params.tap_id).await;
    crate::tap_callback::shutdown_if_idle(Arc::clone(&state.tap)).await;
    outcome.map_err(map_core_error)
}

pub(crate) async fn list(state: &AppState) -> Result<Vec<TapHandle>, RpcError> {
    state.tap.list().await.map_err(map_core_error)
}

fn map_core_error(error: CoreError) -> RpcError {
    match &error {
        CoreError::Tap(message) if message.contains("limit reached") => {
            RpcError::new(TAP_LIMIT, message.clone())
        }
        CoreError::Tap(message)
            if message.contains("disabled") || message.contains("requires a Kubernetes reader") =>
        {
            RpcError::new(TAP_FORBIDDEN, message.clone())
        }
        CoreError::Tap(message) if message.contains("not found") => {
            RpcError::new(NOT_FOUND, message.clone())
        }
        CoreError::Tap(message)
            if message.contains("unsupported")
                || message.contains("required")
                || message.contains("conflict")
                || message.contains("too large") =>
        {
            RpcError::new(INVALID_PARAMS, message.clone())
        }
        CoreError::Kube(kube::Error::Api(status)) if status.code == 404 => {
            RpcError::new(NOT_FOUND, "tap target not found")
        }
        _ => error.to_rpc_error(),
    }
}
