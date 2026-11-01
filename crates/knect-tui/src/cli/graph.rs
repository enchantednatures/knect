//! `knect graph` stub.

use clap::Args;

use super::CliError;

/// Arguments for `knect graph`.
#[derive(Debug, Clone, Args)]
pub struct GraphArgs {
    /// Emit JSON instead of a terminal view.
    #[arg(long)]
    pub json: bool,
    /// Resource as `kind/name` or `apiVersion/kind/namespace/name`.
    pub resource: Option<String>,
    /// Namespace for the resource.
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
}

/// Runs the graph subcommand. Task 15 implements the focused view.
pub fn run(_args: GraphArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented("focused graph view"))
}
