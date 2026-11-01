//! Flagger Canary promote and abort patches.

use knect_proto::events::ActionResult;
use knect_proto::rpc::{FlaggerAction, FlaggerControlParams};
use knect_proto::{Gvk, ResourceId};
use serde_json::{Value, json};

use crate::error::CoreError;
use crate::kube::ClusterReader;

use super::executor::action_result;

const API_VERSION: &str = "flagger.app/v1beta1";
const CANARY_KIND: &str = "Canary";

pub(super) async fn flagger_control(
    reader: &dyn ClusterReader,
    params: FlaggerControlParams,
) -> Result<ActionResult, CoreError> {
    if params.namespace.trim().is_empty() {
        return Err(CoreError::Config("namespace must not be empty".to_owned()));
    }
    if params.canary.trim().is_empty() {
        return Err(CoreError::Config("canary must not be empty".to_owned()));
    }

    let dry_run = params.dry_run.unwrap_or(false);
    let (patch, verb) = patch_for(params.action);
    let canary_gvk = Gvk::new("flagger.app", "v1beta1", CANARY_KIND);
    reader
        .patch_merge(
            &canary_gvk,
            &params.namespace,
            &params.canary,
            patch,
            dry_run,
        )
        .await?;

    let resource =
        ResourceId::namespaced(API_VERSION, CANARY_KIND, &params.namespace, &params.canary);
    let message = format!("canary {}/{} {verb}", params.namespace, params.canary);
    let message = if dry_run {
        format!("dry run: {message}")
    } else {
        message
    };
    Ok(action_result(
        "actions.flagger_control",
        message,
        dry_run,
        Some(resource),
    ))
}

fn patch_for(action: FlaggerAction) -> (Value, &'static str) {
    match action {
        FlaggerAction::Promote => (json!({"spec": {"skipAnalysis": true}}), "promoted"),
        FlaggerAction::Abort => (
            json!({"spec": {"suspend": true, "skipAnalysis": false}}),
            "aborted",
        ),
    }
}
