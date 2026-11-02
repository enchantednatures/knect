//! `webui` scenario.
//!
//! Fetches the rust-embedded Svelte bundle and its assets from a real daemon
//! process. No Node runtime or external web server is involved.

use std::time::Duration;

use crate::harness::{Harness, HarnessError};
use crate::scenarios::{ScenarioError, fixture, protocol};

/// Runs the WebUI scenario against the fixture daemon.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    let (daemon, _harness) = fixture("serving").await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;

    let index = client
        .get(format!("{}/", daemon.http_base()))
        .send()
        .await
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    if !index.status().is_success() {
        return Err(protocol(format!("GET / returned {}", index.status())));
    }
    if index
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.contains("default-src 'self'"))
    {
        return Err(protocol("embedded index is missing its CSP header"));
    }
    let html = index
        .text()
        .await
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    if !html.contains("<title>knect") || !html.contains("id=\"app\"") {
        return Err(protocol("embedded index is not the Svelte shell"));
    }
    let asset = asset_path(&html).ok_or_else(|| protocol("index has no module asset"))?;
    let response = client
        .get(format!("{}{}", daemon.http_base(), asset))
        .send()
        .await
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    if !response.status().is_success() {
        return Err(protocol(format!(
            "GET {asset} returned {}",
            response.status()
        )));
    }
    if response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.contains("javascript"))
    {
        return Err(protocol(
            "embedded JavaScript asset has the wrong MIME type",
        ));
    }
    let missing = client
        .get(format!("{}/assets/does-not-exist.js", daemon.http_base()))
        .send()
        .await
        .map_err(|error| ScenarioError::Harness(HarnessError::Transport(error.to_string())))?;
    if missing.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(protocol(format!(
            "missing asset returned {}, expected 404",
            missing.status()
        )));
    }

    daemon.shutdown().await?;
    Ok(())
}

fn asset_path(html: &str) -> Option<&str> {
    let start = html.find("src=\"")? + 5;
    let rest = &html[start..];
    let end = rest.find('"')?;
    let path = &rest[..end];
    path.starts_with("/assets/").then_some(path)
}
