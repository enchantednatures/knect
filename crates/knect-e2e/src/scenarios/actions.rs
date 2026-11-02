//! `actions` scenario.
//!
//! Proves the daemon fails closed when guarded writes are disabled, including
//! policy dry-run validation that shares the action executor.

use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, fixture, protocol};

/// Runs the actions safety scenario against an isolated fixture daemon.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("serving").await?;

    let traffic = match harness
        .call(
            "actions.set_traffic_split",
            Some(json!({
                "namespace": "knect-e2e",
                "service": "order-processor",
                "splits": [{"revision": "order-processor-00001", "percent": 100}],
                "dry_run": true
            })),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => {
            return Err(protocol("traffic split must be disabled in fixture mode"));
        }
    };
    if !traffic.to_string().contains("disabled") {
        return Err(protocol(format!(
            "set_traffic_split failed for the wrong reason: {traffic}"
        )));
    }

    let flagger = match harness
        .call(
            "actions.flagger_control",
            Some(json!({
                "namespace": "knect-e2e",
                "canary": "order-processor-flagger",
                "action": "promote",
                "dry_run": true
            })),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => {
            return Err(protocol("Flagger control must be disabled in fixture mode"));
        }
    };
    if !flagger.to_string().contains("disabled") {
        return Err(protocol(format!(
            "flagger_control failed for the wrong reason: {flagger}"
        )));
    }

    let validation = match harness
        .call(
            "policy.validate_manifest",
            Some(json!({
                "manifest": "apiVersion: serving.knative.dev/v1\nkind: Service\nmetadata:\n  name: hello\n",
                "namespace": "knect-e2e"
            })),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => {
            return Err(protocol(
                "validation must fail closed in fixture mode",
            ));
        }
    };
    if !validation.to_string().contains("disabled") {
        return Err(protocol(format!(
            "validate_manifest failed for the wrong reason: {validation}"
        )));
    }

    daemon.shutdown().await?;
    Ok(())
}
