//! Fixture mode.
//!
//! Fixture mode never constructs a Kubernetes client. It loads JSON/YAML
//! objects into the store and serves an empty topology until Tasks 4/5 wire the
//! correlation pipeline. A missing or empty directory is a valid empty
//! fixture; the daemon still starts and serves RPC/SSE.

use std::path::{Path, PathBuf};

use chrono::Utc;
use knect_core::store::ResourceStore;
use kube::api::DynamicObject;
use serde::Deserialize;

use crate::AppState;
use crate::app::empty_topology;
use crate::error::DaemonError;

/// CLI fixture selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FixtureMode {
    /// Normal cluster mode.
    #[default]
    Disabled,
    /// Fixture mode. `None` means "fixture mode without a directory".
    Enabled(Option<PathBuf>),
}

impl FixtureMode {
    /// Returns true when fixture mode is selected.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    /// Returns the fixture directory, when supplied.
    #[must_use]
    pub fn directory(&self) -> Option<&Path> {
        match self {
            Self::Disabled | Self::Enabled(None) => None,
            Self::Enabled(Some(path)) => Some(path.as_path()),
        }
    }
}

/// Result of fixture initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureOutcome {
    /// Context name reported by the daemon.
    pub context: String,
    /// Number of objects loaded into the store.
    pub object_count: usize,
}

/// Loads the fixture into the store and installs the empty fixture topology.
///
/// No event is broadcast; fixture startup is silent by contract.
pub async fn initialize(
    state: &AppState,
    mode: &FixtureMode,
) -> Result<FixtureOutcome, DaemonError> {
    let objects = match mode.directory() {
        Some(directory) => load_objects(directory)?,
        None => Vec::new(),
    };
    let object_count = objects.len();
    {
        let mut store = state.store.write().await;
        for object in objects {
            store.upsert(object);
        }
    }
    state.set_topology(empty_topology("fixture", 0));
    state.set_last_resync(Some(Utc::now()));
    Ok(FixtureOutcome {
        context: "fixture".to_owned(),
        object_count,
    })
}

/// Loads every JSON/YAML object under a directory, sorted by path.
pub fn load_objects(directory: &Path) -> Result<Vec<DynamicObject>, DaemonError> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    if !directory.is_dir() {
        return Err(DaemonError::Fixture(format!(
            "{} is not a directory",
            directory.display()
        )));
    }
    let mut files = Vec::new();
    collect_files(directory, &mut files)?;
    files.sort();
    let mut objects = Vec::new();
    for file in files {
        objects.extend(load_file(&file)?);
    }
    Ok(objects)
}

/// Parses one fixture file into one or more dynamic objects.
pub fn load_file(path: &Path) -> Result<Vec<DynamicObject>, DaemonError> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        DaemonError::Fixture(format!("cannot read {}: {error}", path.display()))
    })?;
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => {
            let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
                DaemonError::Fixture(format!("invalid JSON {}: {error}", path.display()))
            })?;
            expand_json(value).map_err(|error| {
                DaemonError::Fixture(format!("invalid object in {}: {error}", path.display()))
            })
        }
        "yaml" | "yml" => {
            let mut objects = Vec::new();
            for document in serde_yaml::Deserializer::from_str(&raw) {
                let value = serde_yaml::Value::deserialize(document).map_err(|error| {
                    DaemonError::Fixture(format!("invalid YAML {}: {error}", path.display()))
                })?;
                let json = serde_json::to_value(value).map_err(|error| {
                    DaemonError::Fixture(format!(
                        "cannot convert YAML in {}: {error}",
                        path.display()
                    ))
                })?;
                objects.extend(expand_json(json).map_err(|error| {
                    DaemonError::Fixture(format!("invalid object in {}: {error}", path.display()))
                })?);
            }
            Ok(objects)
        }
        _ => Ok(Vec::new()),
    }
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), DaemonError> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        DaemonError::Fixture(format!("cannot list {}: {error}", directory.display()))
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            DaemonError::Fixture(format!("cannot read {}: {error}", directory.display()))
        })?;
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            let extension = path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "json" | "yaml" | "yml") {
                files.push(path);
            }
        }
    }
    Ok(())
}

fn expand_json(value: serde_json::Value) -> Result<Vec<DynamicObject>, serde_json::Error> {
    if let Some(items) = list_items(&value) {
        let mut objects = Vec::with_capacity(items.len());
        for item in items {
            objects.push(serde_json::from_value(item.clone())?);
        }
        return Ok(objects);
    }
    Ok(vec![serde_json::from_value(value)?])
}

fn list_items(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(items) = value.as_array() {
        return Some(items);
    }
    let is_list = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind.ends_with("List"));
    if is_list {
        value.get("items").and_then(serde_json::Value::as_array)
    } else {
        None
    }
}

/// Compile-time proof that the fixture loader is sync and cluster-free.
#[must_use]
pub fn empty_store() -> ResourceStore {
    ResourceStore::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_core::config::Config;

    #[tokio::test]
    async fn empty_and_missing_fixtures_start_with_an_empty_topology() {
        let directory = tempfile::tempdir().expect("temp dir");
        let state = AppState::new(Config::default(), true);
        let outcome = initialize(&state, &FixtureMode::Enabled(Some(directory.path().into())))
            .await
            .expect("initialize");
        assert_eq!(outcome.object_count, 0);
        assert_eq!(outcome.context, "fixture");
        assert_eq!(state.topology().context, "fixture");
        assert!(state.topology().nodes.is_empty());
        assert!(state.topology().edges.is_empty());
        assert!(state.last_resync().is_some());

        let missing = directory.path().join("does-not-exist");
        let outcome = initialize(&state, &FixtureMode::Enabled(Some(missing)))
            .await
            .expect("initialize missing");
        assert_eq!(outcome.object_count, 0);
    }

    #[tokio::test]
    async fn json_and_yaml_objects_enter_the_store() {
        let directory = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            directory.path().join("service.json"),
            r#"{"apiVersion":"serving.knative.dev/v1","kind":"Service","metadata":{"name":"hello","namespace":"default"}}"#,
        )
        .expect("write JSON");
        std::fs::write(
            directory.path().join("pod.yaml"),
            "apiVersion: v1\nkind: Pod\nmetadata:\n  name: web\n  namespace: default\n",
        )
        .expect("write YAML");
        std::fs::write(
            directory.path().join("list.json"),
            r#"{"apiVersion":"v1","kind":"List","items":[{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"settings","namespace":"default"}}]}"#,
        )
        .expect("write list");

        let state = AppState::new(Config::default(), true);
        let outcome = initialize(&state, &FixtureMode::Enabled(Some(directory.path().into())))
            .await
            .expect("initialize");
        assert_eq!(outcome.object_count, 3);
        let store = state.store.read().await;
        assert_eq!(store.len(), 3);
        assert_eq!(store.generation(), 3);
        assert!(state.topology().nodes.is_empty());
    }

    #[test]
    fn fixture_mode_helpers_are_explicit() {
        assert!(!FixtureMode::Disabled.is_enabled());
        assert!(FixtureMode::Enabled(None).is_enabled());
        assert_eq!(FixtureMode::Enabled(None).directory(), None);
        assert!(empty_store().is_empty());
    }
}
