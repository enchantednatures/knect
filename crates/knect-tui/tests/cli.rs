#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

fn knect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_knect"))
}

#[test]
fn help_documents_endpoint_precedence_and_ascii_mode() {
    let output = knect().arg("--help").output().expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--socket"));
    assert!(stdout.contains("--daemon-url"));
    assert!(stdout.contains("--ascii"));
    assert!(stdout.contains("precedence"));
}

#[test]
fn graph_subcommand_parses_the_task15_shape() {
    let output = knect()
        .args(["graph", "Service/hello", "-n", "default"])
        .output()
        .expect("run graph");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not implemented yet"),
        "unexpected stderr: {stderr}"
    );
}
