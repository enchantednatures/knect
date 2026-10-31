//! `knect install-k9s-plugin` stub.

use std::path::PathBuf;

use clap::Args;

use super::CliError;
use crate::cli::install;

/// Arguments for `knect install-k9s-plugin`.
#[derive(Debug, Clone, Args)]
pub struct InstallK9sPluginArgs {
    /// Destination directory.
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Overwrite an existing plugin file.
    #[arg(long)]
    pub force: bool,
}

/// Delegates to the install stub.
pub fn run(args: InstallK9sPluginArgs) -> Result<(), CliError> {
    install::run(&args)
}
