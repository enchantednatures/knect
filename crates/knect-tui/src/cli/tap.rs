//! `knect tap` stub.

use clap::Args;

use super::CliError;

/// Arguments for `knect tap`.
#[derive(Debug, Clone, Args)]
pub struct TapArgs {
    /// Broker or Trigger target.
    pub target: Option<String>,
    /// Follow events until interrupted.
    #[arg(long)]
    pub follow: bool,
}

/// Runs the tap subcommand. Task 14 replaces this with the live tap view.
pub fn run(_args: TapArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented("tap view"))
}
