//! `nvim` scenario.
//!
//! Verifies the checked-in plugin surface. A full Plenary run is exercised by
//! `hack/nvim-test.sh` and the dedicated nvim workflow; this scenario keeps the
//! E2E registry honest without requiring Neovim on the Kind runner.

use std::path::Path;
use std::process::Command;

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, protocol};

/// Runs the nvim packaging checks.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "nvim/plugin/knect.lua",
        "nvim/lua/knect/init.lua",
        "nvim/doc/knect.txt",
    ] {
        let path = root.join(relative);
        if !path.is_file() {
            return Err(protocol(format!("missing Neovim plugin file {relative}")));
        }
    }
    let commands = std::fs::read_to_string(root.join("nvim/lua/knect/init.lua"))
        .map_err(|error| protocol(format!("cannot read the Neovim command module: {error}")))?;
    for command in ["Knect", "KnectServices", "KnectBrokers", "KnectDiff"] {
        if !commands.contains(command) {
            return Err(protocol(format!(
                "Neovim plugin does not register :{command}"
            )));
        }
    }

    if Command::new("nvim").arg("--version").output().is_ok() {
        let status = Command::new("nvim")
            .arg("--headless")
            .arg("-u")
            .arg("NONE")
            .arg("-c")
            .arg(format!("set runtimepath+={}", root.join("nvim").display()))
            .arg("-c")
            .arg("lua require('knect')")
            .arg("-c")
            .arg("qa!")
            .status()
            .map_err(|error| protocol(format!("cannot launch nvim: {error}")))?;
        if !status.success() {
            return Err(protocol("Neovim could not load the knect module"));
        }
    }

    Ok(())
}
