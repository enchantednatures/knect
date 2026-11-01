//! `knect install-k9s-plugin` arguments.

use std::path::{Path, PathBuf};

use clap::Args;

use super::CliError;
use crate::cli::install;

/// Arguments for `knect install-k9s-plugin`.
#[derive(Debug, Clone, Args)]
pub struct InstallK9sPluginArgs {
    /// Destination directory (defaults to the k9s config directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Replace an unparseable plugin file instead of failing.
    #[arg(long)]
    pub force: bool,
    /// Print the merged YAML without writing or backing up.
    #[arg(long)]
    pub dry_run: bool,
}

/// Installs the plugin shortcuts.
pub fn run(args: InstallK9sPluginArgs, socket: Option<&Path>) -> Result<(), CliError> {
    install::run(&args, socket)
}
