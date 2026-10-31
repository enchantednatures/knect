//! `knect graph` stub.

use clap::Args;

use super::CliError;

/// Arguments for `knect graph`.
#[derive(Debug, Clone, Args)]
pub struct GraphArgs {
    /// Emit JSON instead of a terminal view.
    #[arg(long)]
    pub json: bool,
}

/// Runs the graph subcommand. Task 13 replaces this with the TUI view.
pub fn run(_args: GraphArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented("graph view"))
}
