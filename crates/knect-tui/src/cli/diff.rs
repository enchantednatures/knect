//! `knect diff` stub.

use clap::{Args, ValueEnum};

use super::CliError;

/// Diff rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiffModeCli {
    /// Compare user intent, Knative defaults, and policy mutators.
    ThreeWay,
    /// Group ownership by manager.
    Managers,
}

/// Arguments for `knect diff`.
#[derive(Debug, Clone, Args)]
pub struct DiffArgs {
    /// Resource name or `apiVersion/kind/namespace/name` identifier.
    pub resource: Option<String>,
    /// Diff rendering mode.
    #[arg(long, value_enum, default_value = "three-way")]
    pub mode: DiffModeCli,
}

/// Runs the diff subcommand. Task 15 replaces this with the modal view.
pub fn run(_args: DiffArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented("mutation diff"))
}
