//! `tap` scenario.
//!
//! Fixture mode must fail closed because it has no Kubernetes reader. The live
//! tap lifecycle is covered by the core manager tests and by the live daemon
//! suite when a cluster-backed socket is supplied.

use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, fixture, protocol};

/// Runs the tap safety scenario against an isolated fixture daemon.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("eventing").await?;

    let list = harness.call("tap.list", Some(json!({}))).await?;
    if list.as_array().is_none_or(|taps| !taps.is_empty()) {
        return Err(protocol("fixture daemon reported active taps"));
    }

    let error = match harness
        .call(
            "tap.start",
            Some(json!({
                "target": {"kind": "broker", "namespace": "knect-e2e", "name": "kafka-broker"},
                "max_events": 10,
                "ttl_secs": 30
            })),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => return Err(protocol("fixture mode must reject tap.start")),
    };
    if !error.to_string().contains("fixture") {
        return Err(protocol(format!(
            "tap.start failed for the wrong reason: {error}"
        )));
    }

    let stopped = harness
        .call("tap.stop", Some(json!({"tap_id": "tap-missing"})))
        .await?;
    if stopped["removed"] != false {
        return Err(protocol(
            "tap.stop on a missing tap did not return removed=false",
        ));
    }

    daemon.shutdown().await?;
    Ok(())
}
