//! YAML rendering and line/column indexing for mutation attribution.
//!
//! Documents are rendered with `serde_yaml` and then re-parsed with
//! `marked-yaml` so every mapping key and sequence item keeps its source
//! marker. The index maps JSON Pointer paths to one-based line and column
//! numbers, and records every path present on a physical line so callers can
//! pick the most specific one.

use std::collections::BTreeMap;

use knect_proto::FieldPath;
use marked_yaml::{Marker, Node};
use serde_json::Value;

use crate::mutations::pointer::join;

/// A document that could not be rendered or re-parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum YamlIndexError {
    /// `serde_yaml` rejected the document.
    #[error("cannot render YAML: {0}")]
    Render(String),
    /// `marked-yaml` rejected the rendered document.
    #[error("cannot parse rendered YAML: {0}")]
    Parse(String),
}

/// Line and column information for every path in a rendered document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct YamlIndex {
    spans: BTreeMap<String, (u32, u32)>,
    paths_by_line: BTreeMap<u32, Vec<String>>,
    line_count: u32,
}

impl YamlIndex {
    /// Parses `yaml` and indexes every path.
    pub fn build(yaml: &str) -> Result<Self, YamlIndexError> {
        let node = marked_yaml::parse_yaml(0, yaml)
            .map_err(|error| YamlIndexError::Parse(error.to_string()))?;
        let mut index = Self {
            line_count: u32::try_from(yaml.lines().count()).unwrap_or(u32::MAX),
            ..Self::default()
        };
        index.walk(&node, "");
        Ok(index)
    }

    /// Returns the one-based line and column for a path.
    #[must_use]
    pub fn span(&self, path: &FieldPath) -> Option<(u32, u32)> {
        self.spans.get(path.as_str()).copied()
    }

    /// Returns every path whose key or item begins on `line`.
    #[must_use]
    pub fn paths_on_line(&self, line: u32) -> &[String] {
        self.paths_by_line.get(&line).map_or(&[], Vec::as_slice)
    }

    /// Number of physical lines in the rendered document.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.line_count
    }

    fn walk(&mut self, node: &Node, pointer: &str) {
        if let Some((line, column)) = node.span().start().and_then(marker) {
            self.record(pointer, line, column);
        }
        match node {
            Node::Mapping(mapping) => {
                for (key, value) in mapping.iter() {
                    let child = join(pointer, key.as_str());
                    if let Some((line, column)) = key.span().start().and_then(marker) {
                        self.record(&child, line, column);
                    }
                    self.walk(value, &child);
                }
            }
            Node::Sequence(sequence) => {
                for (index, item) in sequence.iter().enumerate() {
                    let child = join(pointer, &index.to_string());
                    self.walk(item, &child);
                }
            }
            Node::Scalar(_) => {}
        }
    }

    fn record(&mut self, pointer: &str, line: u32, column: u32) {
        self.spans
            .entry(pointer.to_owned())
            .or_insert((line, column));
        let paths = self.paths_by_line.entry(line).or_default();
        if !paths.iter().any(|existing| existing == pointer) {
            paths.push(pointer.to_owned());
        }
    }
}

/// Renders a JSON document as YAML.
pub fn render_yaml(document: &Value) -> Result<String, YamlIndexError> {
    serde_yaml::to_string(document).map_err(|error| YamlIndexError::Render(error.to_string()))
}

fn marker(marker: &Marker) -> Option<(u32, u32)> {
    Some((
        u32::try_from(marker.line()).ok()?,
        u32::try_from(marker.column()).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn index(document: &Value) -> (String, YamlIndex) {
        let yaml = render_yaml(document).expect("document renders");
        let index = YamlIndex::build(&yaml).expect("rendered YAML parses");
        (yaml, index)
    }

    #[test]
    fn mapping_keys_and_list_items_keep_their_lines() {
        let document = json!({
            "apiVersion": "serving.knative.dev/v1",
            "kind": "Service",
            "metadata": {"name": "hello"},
            "spec": {"template": {"spec": {"containers": [{"image": "nginx"}]}}}
        });
        let (yaml, index) = index(&document);
        let image = FieldPath::new("/spec/template/spec/containers/0/image");
        let (line, column) = index.span(&image).expect("image path is indexed");
        assert_eq!(
            yaml.lines().nth(line as usize - 1).map(str::trim),
            Some("- image: nginx")
        );
        assert!(column > 1);
        assert!(!index.paths_on_line(line).is_empty());
        assert!(
            index
                .paths_on_line(line)
                .iter()
                .any(|path| path == image.as_str())
        );
        assert_eq!(index.line_count(), yaml.lines().count() as u32);
    }

    #[test]
    fn quoted_keys_and_missing_paths_are_handled() {
        let document = json!({
            "metadata": {"annotations": {"example.com/key": "value"}}
        });
        let (_yaml, index) = index(&document);
        assert!(
            index
                .span(&FieldPath::new("/metadata/annotations/example.com~1key"))
                .is_some()
        );
        assert!(
            index
                .span(&FieldPath::new("/metadata/annotations/absent"))
                .is_none()
        );
    }

    #[test]
    fn repeated_indexing_is_deterministic() {
        let document = json!({"spec": {"traffic": [{"percent": 90}, {"percent": 10}]}});
        let (yaml, first) = index(&document);
        let second = YamlIndex::build(&yaml).expect("same YAML parses");
        assert_eq!(first, second);
        assert_eq!(
            first.span(&FieldPath::new("/spec/traffic/1/percent")),
            second.span(&FieldPath::new("/spec/traffic/1/percent"))
        );
    }
}
