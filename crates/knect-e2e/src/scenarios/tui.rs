//! `tui` scenario.
//!
//! Exercises the one-shot CLI inspection commands and the k9s plugin generator
//! against a real fixture daemon.

use std::fs;
use std::process::Command;
use std::time::Instant;

use crate::harness::{Harness, knect_binary};
use crate::scenarios::{ScenarioError, fixture, protocol};

/// Runs the tui/k9s scenario against the isolated mutation fixture.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, _harness) = fixture("mutations").await?;
    let binary = knect_binary().await?;
    let socket = daemon.socket();

    let started = Instant::now();
    let graph = Command::new(&binary)
        .arg("--socket")
        .arg(socket)
        .arg("graph")
        .arg("ksvc/order-processor")
        .arg("-n")
        .arg("knect-e2e")
        .arg("--json")
        .output()
        .map_err(|error| protocol(format!("cannot run knect graph: {error}")))?;
    let elapsed = started.elapsed();
    if !graph.status.success() {
        return Err(protocol(format!(
            "knect graph failed: {}",
            String::from_utf8_lossy(&graph.stderr)
        )));
    }
    let focused: serde_json::Value = serde_json::from_slice(&graph.stdout)
        .map_err(|error| protocol(format!("knect graph returned invalid JSON: {error}")))?;
    if focused["resource"]["name"] != "order-processor"
        || focused["resource"]["category"] != "k_service"
    {
        return Err(protocol("knect graph focused the wrong resource"));
    }
    if elapsed.as_secs() > 5 {
        return Err(protocol(format!(
            "knect graph took {elapsed:?}, expected a one-shot response"
        )));
    }

    let diff = Command::new(&binary)
        .arg("--socket")
        .arg(socket)
        .arg("diff")
        .arg("ksvc/order-processor-mutated")
        .arg("-n")
        .arg("knect-e2e")
        .arg("--json")
        .output()
        .map_err(|error| protocol(format!("cannot run knect diff: {error}")))?;
    if !diff.status.success() {
        return Err(protocol(format!(
            "knect diff failed: {}",
            String::from_utf8_lossy(&diff.stderr)
        )));
    }
    let diff: serde_json::Value = serde_json::from_slice(&diff.stdout)
        .map_err(|error| protocol(format!("knect diff returned invalid JSON: {error}")))?;
    if diff["lines"].as_array().is_none_or(Vec::is_empty) {
        return Err(protocol("knect diff returned no attributed lines"));
    }

    let directory = std::env::temp_dir().join(format!("knect-e2e-k9s-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let plugin = Command::new(&binary)
        .arg("--socket")
        .arg(socket)
        .arg("install-k9s-plugin")
        .arg("--dir")
        .arg(&directory)
        .arg("--dry-run")
        .output()
        .map_err(|error| protocol(format!("cannot run knect install-k9s-plugin: {error}")))?;
    if !plugin.status.success() {
        return Err(protocol(format!(
            "install-k9s-plugin failed: {}",
            String::from_utf8_lossy(&plugin.stderr)
        )));
    }
    let yaml = String::from_utf8_lossy(&plugin.stdout);
    for expected in [
        "knect-topology",
        "knect-diff",
        "Shift-T",
        "Shift-M",
        "ksvc",
        "broker",
        "trigger",
    ] {
        if !yaml.contains(expected) {
            return Err(protocol(format!(
                "generated k9s plugin is missing {expected}"
            )));
        }
    }
    if !yaml.contains(&socket.to_string_lossy().into_owned()) {
        return Err(protocol(
            "generated k9s plugin does not point at the daemon socket",
        ));
    }
    let _ = fs::remove_dir_all(&directory);

    println!(
        "knect graph focused ksvc/order-processor in {} ms",
        elapsed.as_millis()
    );

    daemon.shutdown().await?;
    Ok(())
}
