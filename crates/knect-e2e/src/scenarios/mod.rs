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

use crate::harness::{Harness, HarnessError};

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
