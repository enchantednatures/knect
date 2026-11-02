//! `mutations` scenario.
//!
//! Exercises the three-way mutation diff through the real daemon RPC surface.

use knect_proto::mutation::{MutationDiff, MutationSource};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::json;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, fixture, protocol, require_node};

/// Runs the mutations scenario against the isolated mutation fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, harness) = fixture("mutations").await?;
    let topology = harness.topology().await?;
    require_node(&topology, NodeCategory::KService, "order-processor-mutated")?;

    let id = ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Service",
        "knect-e2e",
        "order-processor-mutated",
    );
    let three_way: MutationDiff = serde_json::from_value(
        harness
            .call(
                "mutations.get_diff",
                Some(json!({"id": id, "mode": "three_way"})),
            )
            .await?,
    )
    .map_err(|error| protocol(format!("invalid three-way diff: {error}")))?;

    let user = three_way
        .user_intent
        .as_deref()
        .ok_or_else(|| protocol("three-way diff has no user-intent pane"))?;
    if !user.contains("image: nginxinc/nginx-unprivileged:1.27.4") {
        return Err(protocol("user-intent pane does not contain the image"));
    }
    if user.contains("managedFields") {
        return Err(protocol("user-intent pane contains managedFields"));
    }

    let defaults = three_way
        .knative_defaults
        .as_deref()
        .ok_or_else(|| protocol("three-way diff has no Knative-defaults pane"))?;
    if !defaults.contains("idleTimeoutSeconds: 300")
        || !defaults.contains("client.knative.dev/user-image")
    {
        return Err(protocol(
            "Knative-defaults pane does not contain webhook fields",
        ));
    }

    let policy = three_way
        .policy_mutators
        .as_deref()
        .ok_or_else(|| protocol("three-way diff has no policy pane"))?;
    if !policy.contains("knect.dev/managed") || !policy.contains("sidecar.istio.io/inject") {
        return Err(protocol(
            "policy pane does not contain Kyverno/Istio mutations",
        ));
    }

    let kyverno = three_way
        .lines
        .iter()
        .find(|line| {
            line.owner.as_deref() == Some("kyverno")
                && line.side == MutationSource::PolicyMutators
                && line.text.contains("knect.dev/managed")
        })
        .ok_or_else(|| protocol("no Kyverno-owned policy line"))?;
    if kyverno.line_number == 0 || kyverno.path.is_none() {
        return Err(protocol("Kyverno line lacks a line number or field path"));
    }

    let webhook = three_way
        .lines
        .iter()
        .find(|line| {
            line.owner.as_deref() == Some("webhook.serving.knative.dev")
                && line.side == MutationSource::KnativeDefaults
        })
        .ok_or_else(|| protocol("no Knative webhook-owned default line"))?;
    if webhook.line_number == 0 {
        return Err(protocol("Knative webhook line lacks a line number"));
    }

    let managers: MutationDiff = serde_json::from_value(
        harness
            .call(
                "mutations.get_diff",
                Some(json!({"id": id, "mode": "managers"})),
            )
            .await?,
    )
    .map_err(|error| protocol(format!("invalid manager diff: {error}")))?;
    let owners: Vec<&str> = managers
        .lines
        .iter()
        .filter_map(|line| line.owner.as_deref())
        .collect();
    for expected in [
        "kubectl-client-side-apply",
        "kyverno",
        "istiod",
        "flagger",
        "controller",
    ] {
        if !owners.contains(&expected) {
            return Err(protocol(format!(
                "manager diff is missing owner {expected}"
            )));
        }
    }
    if managers.lines.iter().all(|line| line.line_number == 0) {
        return Err(protocol("manager diff has no YAML line numbers"));
    }

    daemon.shutdown().await?;
    Ok(())
}
