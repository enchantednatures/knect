//! Scenario registry.
//!
//! Every later task owns one module here. Foundation scenarios print
//! `pending` and succeed so the harness, CLI, and exit-code contract are
//! exercised before real assertions land.

pub mod actions;
pub mod daemon;
pub mod eventing;
pub mod links;
pub mod mutations;
pub mod nvim;
pub mod operators;
pub mod policy;
pub mod serving;
pub mod storage;
pub mod tap;
pub mod tui;
pub mod webui;

use knect_proto::NodeCategory;
use knect_proto::graph::{Edge, EdgeKind, ResourceNode, Topology};

use crate::harness::{FixtureDaemon, Harness, HarnessError, repo_fixture_dir};

/// Registered scenario names, in execution order.
pub const SCENARIOS: &[&str] = &[
    "daemon",
    "serving",
    "eventing",
    "links",
    "operators",
    "storage",
    "mutations",
    "policy",
    "actions",
    "tap",
    "tui",
    "webui",
    "nvim",
];

/// Scenario failure.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// Unknown scenario name.
    #[error("unknown scenario '{0}'")]
    Unknown(String),
    /// Harness failure.
    #[error(transparent)]
    Harness(#[from] HarnessError),
}

/// Prints the foundation status line for one scenario.
pub fn pending(name: &str) {
    println!("scenario {name}: pending");
}

/// Spawns a disposable fixture daemon for one scenario directory.
///
/// Keeping each scenario on its own daemon makes the assertions deterministic
/// and avoids leaking state between scenarios.
pub async fn fixture(name: &str) -> Result<(FixtureDaemon, Harness), ScenarioError> {
    let directory = repo_fixture_dir().join(name);
    let daemon = FixtureDaemon::spawn(&directory).await?;
    let harness = Harness::new(None, Some(daemon.socket()))?;
    Ok((daemon, harness))
}

/// Builds a scenario protocol failure.
pub fn protocol(message: impl Into<String>) -> ScenarioError {
    ScenarioError::Harness(HarnessError::Protocol(message.into()))
}

/// Finds a node by category and name or fails with the requested identity.
pub fn require_node<'a>(
    topology: &'a Topology,
    category: NodeCategory,
    name: &str,
) -> Result<&'a ResourceNode, ScenarioError> {
    topology
        .nodes
        .iter()
        .find(|node| node.category == category && node.name == name)
        .ok_or_else(|| protocol(format!("missing {category:?} node {name}")))
}

/// Finds one edge by endpoints and kind or fails with the requested identity.
pub fn require_edge<'a>(
    topology: &'a Topology,
    from: &str,
    to: &str,
    kind: EdgeKind,
) -> Result<&'a Edge, ScenarioError> {
    topology
        .edges
        .iter()
        .find(|edge| edge.from.name == from && edge.to.name == to && edge.kind == kind)
        .ok_or_else(|| protocol(format!("missing edge {from} -{kind:?}-> {to}")))
}

/// Returns the badge labels for a node.
pub fn badge_labels(node: &ResourceNode) -> Vec<&str> {
    node.badges
        .iter()
        .map(|badge| badge.label.as_str())
        .collect()
}

/// Runs one scenario by name.
pub async fn run_scenario(name: &str, harness: &Harness) -> Result<(), ScenarioError> {
    match name {
        "daemon" => daemon::run(harness).await,
        "serving" => serving::run(harness).await,
        "eventing" => eventing::run(harness).await,
        "links" => links::run(harness).await,
        "operators" => operators::run(harness).await,
        "storage" => storage::run(harness).await,
        "mutations" => mutations::run(harness).await,
        "policy" => policy::run(harness).await,
        "actions" => actions::run(harness).await,
        "tap" => tap::run(harness).await,
        "tui" => tui::run(harness).await,
        "webui" => webui::run(harness).await,
        "nvim" => nvim::run(harness).await,
        other => Err(ScenarioError::Unknown(other.to_owned())),
    }
}

/// Runs every registered scenario, stopping at the first failure.
pub async fn run_all(harness: &Harness) -> Result<(), ScenarioError> {
    for name in SCENARIOS {
        run_scenario(name, harness).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn all_registered_scenarios_pass() {
        let harness =
            Harness::new(None, Some(Path::new("/tmp/knect-e2e-unused.sock"))).expect("harness");
        run_all(&harness).await.expect("skeleton scenarios pass");
    }

    #[tokio::test]
    async fn unknown_scenario_is_typed() {
        let harness =
            Harness::new(None, Some(Path::new("/tmp/knect-e2e-unused.sock"))).expect("harness");
        assert!(matches!(
            run_scenario("nope", &harness).await,
            Err(ScenarioError::Unknown(_))
        ));
    }
}
