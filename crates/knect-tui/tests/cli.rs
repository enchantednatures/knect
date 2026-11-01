#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::Value;
use tokio::process::Command;

fn knect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_knect"))
}

#[tokio::test]
async fn help_documents_endpoint_precedence_and_ascii_mode() {
    let output = knect().arg("--help").output().await.expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--socket"));
    assert!(stdout.contains("--daemon-url"));
    assert!(stdout.contains("--ascii"));
    assert!(stdout.contains("precedence"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graph_prints_a_deterministic_focus_and_json() {
    let server = support::start_stub_daemon().await;
    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "graph",
            "ksvc/hello",
            "-n",
            "default",
        ])
        .output()
        .await
        .expect("run graph");
    assert!(
        output.status.success(),
        "graph failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("resource: Service default/hello"),
        "{stdout}"
    );
    assert!(stdout.contains("outgoing (1):"), "{stdout}");
    assert!(stdout.contains("incoming (1):"), "{stdout}");

    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "graph",
            "ksvc/hello",
            "-n",
            "default",
            "--json",
        ])
        .output()
        .await
        .expect("run graph json");
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(value["resource"]["name"], "hello");
    assert!(
        value["outgoing"]
            .as_array()
            .is_some_and(|edges| !edges.is_empty())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graph_exits_non_zero_for_a_missing_resource() {
    let server = support::start_stub_daemon().await;
    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "graph",
            "ksvc/missing",
            "-n",
            "default",
        ])
        .output()
        .await
        .expect("run graph");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("resource not found"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_prints_all_three_panes_with_owners() {
    let server = support::start_stub_daemon().await;
    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "diff",
            "ksvc/hello",
            "-n",
            "default",
        ])
        .output()
        .await
        .expect("run diff");
    assert!(
        output.status.success(),
        "diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "== user_intent ==",
        "== knative_defaults ==",
        "== policy_mutators ==",
        "knative-serving-controller",
        "istio-sidecar-injector",
    ] {
        assert!(stdout.contains(expected), "missing {expected} in {stdout}");
    }

    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "diff",
            "ksvc/hello",
            "-n",
            "default",
            "--json",
        ])
        .output()
        .await
        .expect("run diff json");
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(value["resource"]["name"], "hello");
    assert!(
        value["lines"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tap_lists_and_starts_without_hanging() {
    let server = support::start_stub_daemon().await;
    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "tap",
            "--list",
            "--json",
        ])
        .output()
        .await
        .expect("run tap list");
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).expect("json"),
        Value::Array(Vec::new())
    );

    let output = knect()
        .args([
            "--daemon-url",
            server.url.as_str(),
            "tap",
            "broker/orders",
            "-n",
            "default",
        ])
        .output()
        .await
        .expect("run tap start");
    assert!(
        output.status.success(),
        "tap failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("stub-tap"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn install_k9s_plugin_writes_and_is_idempotent() {
    let directory = tempfile::tempdir().expect("temp");
    let path = directory.path().join("plugins.yaml");
    let output = knect()
        .args([
            "install-k9s-plugin",
            "--dir",
            directory.path().to_str().expect("path"),
        ])
        .output()
        .await
        .expect("run install");
    assert!(output.status.success());
    assert!(path.exists());
    let first = std::fs::read_to_string(&path).expect("read plugins");
    assert!(first.contains("knect-topology"));
    assert!(first.contains("Shift-T"));
    let output = knect()
        .args([
            "install-k9s-plugin",
            "--dir",
            directory.path().to_str().expect("path"),
        ])
        .output()
        .await
        .expect("run install again");
    assert!(output.status.success());
    assert_eq!(first, std::fs::read_to_string(&path).expect("read plugins"));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("already up to date"),
        "second install was not idempotent"
    );
}
