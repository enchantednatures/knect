//! k9s plugin installation stub.

use super::CliError;
use super::k9s::InstallK9sPluginArgs;

/// Installs the k9s plugin. Task 15 writes the plugin YAML and tests it.
pub fn run(_args: &InstallK9sPluginArgs) -> Result<(), CliError> {
    Err(CliError::NotImplemented("install-k9s-plugin"))
}
