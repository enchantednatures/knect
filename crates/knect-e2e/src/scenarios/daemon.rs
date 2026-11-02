//! `daemon` scenario.
//!
//! Spawns a fixture daemon, drives the real UDS JSON-RPC and HTTP SSE
//! surfaces, and verifies the fixture Serving/Eventing topology plus graceful
//! socket cleanup.

use std::time::{Duration, Instant};

use knect_proto::NodeCategory;
use serde_json::json;

use crate::harness::{FixtureDaemon, Harness, HarnessError, repo_fixture_dir};
use crate::scenarios::ScenarioError;

/// Runs the daemon scenario against a real fixture daemon process.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    crate::ensure_crypto_provider();
    let fixture = repo_fixture_dir();
    let daemon = FixtureDaemon::spawn(&fixture).await?;
    let socket = daemon.socket().to_path_buf();
    let http_base = daemon.http_base().to_owned();
    let uds = Harness::new(None, Some(&socket))?;

    let status = uds
        .call("daemon.status", Some(json!({})))
        .await
        .map_err(ScenarioError::Harness)?;
    if status["context"] != "fixture" {
        return Err(protocol(format!(
            "daemon.status context is {:?}, expected fixture",
            status["context"]
        )));
    }
    if status["generation"].as_u64().unwrap_or(0) == 0 {
        return Err(protocol("daemon.status generation is zero"));
    }

    let topology = uds.topology().await.map_err(ScenarioError::Harness)?;
    for category in [
        NodeCategory::KService,
        NodeCategory::Revision,
        NodeCategory::Broker,
        NodeCategory::Trigger,
    ] {
        if !topology.nodes.iter().any(|node| node.category == category) {
            return Err(protocol(format!("topology has no {category:?} node")));
        }
    }
    if topology.edges.is_empty() {
        return Err(protocol("topology has no edges"));
    }

    let service = topology
        .nodes
        .iter()
        .find(|node| node.category == NodeCategory::KService)
        .ok_or_else(|| protocol("no KService node for detail lookup"))?;
    let details = uds
        .call(
            "graph.get_node_details",
            Some(json!({"id": service.id, "include_yaml": true})),
        )
        .await
        .map_err(ScenarioError::Harness)?;
    if details["yaml_redacted"] != true {
        return Err(protocol("node details did not mark YAML as redacted"));
    }
    if !details["yaml"].is_string() {
        return Err(protocol("node details did not include YAML"));
    }
    for array in ["links", "policies", "storage", "mutations"] {
        if details[array].as_array().is_none() {
            return Err(protocol(format!("node details {array} must be an array")));
        }
    }
    if details["policies"].as_array().is_none_or(Vec::is_empty) {
        return Err(protocol("node details policies must be populated"));
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    let mut response = client
        .get(format!("{http_base}/events"))
        .send()
        .await
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    if !response.status().is_success() {
        return Err(protocol(format!(
            "GET /events returned {}",
            response.status()
        )));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buffer = Vec::new();
    let mut saw_snapshot = false;
    while Instant::now() < deadline && !saw_snapshot {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                buffer.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&buffer).contains("event: snapshot") {
                    saw_snapshot = true;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                return Err(protocol(format!("SSE stream failed: {error}")));
            }
            Err(_) => break,
        }
    }
    if !saw_snapshot {
        return Err(protocol("GET /events did not deliver a snapshot event"));
    }

    daemon.shutdown().await.map_err(ScenarioError::Harness)?;
    if socket.exists() {
        return Err(protocol(format!(
            "socket {} still exists after shutdown",
            socket.display()
        )));
    }
    Ok(())
}

fn protocol(message: impl Into<String>) -> ScenarioError {
    ScenarioError::Harness(HarnessError::Protocol(message.into()))
}
