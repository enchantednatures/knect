//! Parser for the Kubernetes `FieldsV1` managed-fields trie.
//!
//! A manager's field set is a JSON trie. Each key is a reference token:
//!
//! - `f:<name>` is a struct field or a plain map key;
//! - `k:<json>` is a keyed (associative) list item, for example
//!   `k:{"name":"app"}`;
//! - `v:<json>` is a set item identified by its JSON value;
//! - `i:<index>` is a list index;
//! - `.` represents the node itself.
//!
//! A key whose value is an empty object is an owned field. A key with children
//! only describes more specific ownership below that node, so the parser emits
//! leaves (and explicit `.` markers) rather than every intermediate node.

use knect_proto::FieldPath;
use serde_json::Value;

use crate::mutations::pointer::join;

/// A malformed `FieldsV1` trie.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldsV1Error {
    /// The trie root is not a JSON object.
    #[error("FieldsV1 must be a JSON object")]
    NotAnObject,
    /// A trie child is not a JSON object.
    #[error("FieldsV1 entry '{key}' must map to an object")]
    EntryNotAnObject {
        /// Offending key.
        key: String,
    },
    /// The `.` marker carried children instead of an empty object.
    #[error("FieldsV1 entry '{key}' must map to an empty object")]
    SelfEntryNotEmpty {
        /// Offending key.
        key: String,
    },
    /// The key does not use one of the four documented prefixes.
    #[error("FieldsV1 key '{key}' has an unsupported prefix")]
    UnknownKey {
        /// Offending key.
        key: String,
    },
    /// An `f:` key has an empty field name.
    #[error("FieldsV1 field name must not be empty")]
    EmptyFieldName,
    /// An `i:` key is not an unsigned integer.
    #[error("FieldsV1 list index '{value}' is invalid")]
    InvalidIndex {
        /// Offending index text.
        value: String,
    },
    /// A `k:` payload is not valid JSON.
    #[error("FieldsV1 key payload '{value}' is not valid JSON: {reason}")]
    InvalidKeyPayload {
        /// Offending payload.
        value: String,
        /// Parser diagnostic.
        reason: String,
    },
    /// A `v:` payload is not valid JSON.
    #[error("FieldsV1 value payload '{value}' is not valid JSON: {reason}")]
    InvalidValuePayload {
        /// Offending payload.
        value: String,
        /// Parser diagnostic.
        reason: String,
    },
    /// A `k:` payload decoded to something other than an object.
    #[error("FieldsV1 key payload must be a JSON object")]
    KeyPayloadNotAnObject,
    /// A `k:` payload decoded to an empty object.
    #[error("FieldsV1 key payload must not be empty")]
    EmptyKeyPayload,
}

/// One decoded reference token from a `FieldsV1` trie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// `f:<name>`.
    Field(String),
    /// `k:<json object>`.
    Key(Value),
    /// `v:<json value>`.
    Value(Value),
    /// `i:<index>`.
    Index(u32),
}

/// An owned path that has not been resolved against a document yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPath {
    /// Reference tokens in order.
    pub segments: Vec<PathSegment>,
}

impl ParsedPath {
    /// Best-effort JSON Pointer using the raw JSON text for `k:`/`v:` tokens.
    #[must_use]
    pub fn best_effort_pointer(&self) -> FieldPath {
        let mut pointer = String::new();
        for segment in &self.segments {
            let token = match segment {
                PathSegment::Field(name) => name.clone(),
                PathSegment::Index(index) => index.to_string(),
                PathSegment::Key(value) | PathSegment::Value(value) => value.to_string(),
            };
            pointer = join(&pointer, &token);
        }
        FieldPath::new(pointer)
    }

    /// Resolves `k:` and `v:` tokens against a document.
    ///
    /// Keyed list items are matched by their key fields and set items by exact
    /// JSON value. When either cannot be found the result is marked unresolved
    /// and carries only the best-effort pointer.
    #[must_use]
    pub fn resolve(&self, root: &Value) -> ResolvedPath {
        let mut pointer = String::new();
        let mut segments = Vec::with_capacity(self.segments.len());
        let mut current = Some(root);
        let mut resolved = true;

        for segment in &self.segments {
            match segment {
                PathSegment::Field(name) => {
                    pointer = join(&pointer, name);
                    segments.push(ConcreteSegment::Field(name.clone()));
                    current = current.and_then(|value| value.get(name));
                }
                PathSegment::Index(index) => {
                    let index = *index as usize;
                    pointer = join(&pointer, &index.to_string());
                    segments.push(ConcreteSegment::Index(index));
                    current = current.and_then(|value| value.get(index));
                }
                PathSegment::Key(key) => {
                    let found = current
                        .and_then(Value::as_array)
                        .and_then(|items| items.iter().position(|item| key_matches(key, item)));
                    match found {
                        Some(index) => {
                            pointer = join(&pointer, &index.to_string());
                            segments.push(ConcreteSegment::Index(index));
                            current = current
                                .and_then(Value::as_array)
                                .and_then(|items| items.get(index));
                        }
                        None => {
                            resolved = false;
                            pointer = join(&pointer, &key.to_string());
                            current = None;
                        }
                    }
                }
                PathSegment::Value(value) => {
                    let found = current
                        .and_then(Value::as_array)
                        .and_then(|items| items.iter().position(|item| item == value));
                    match found {
                        Some(index) => {
                            pointer = join(&pointer, &index.to_string());
                            segments.push(ConcreteSegment::Index(index));
                            current = current
                                .and_then(Value::as_array)
                                .and_then(|items| items.get(index));
                        }
                        None => {
                            resolved = false;
                            pointer = join(&pointer, &value.to_string());
                            current = None;
                        }
                    }
                }
            }
        }

        if !resolved {
            segments.clear();
        }
        ResolvedPath {
            pointer: FieldPath::new(pointer),
            segments,
            resolved,
        }
    }
}

/// A resolved reference token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConcreteSegment {
    /// Object key.
    Field(String),
    /// Array index.
    Index(usize),
}

/// An owned path resolved against a concrete document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// JSON Pointer with keyed list items resolved to array indexes when
    /// possible.
    pub pointer: FieldPath,
    /// Concrete tokens, or empty when a keyed/set item was not found.
    pub segments: Vec<ConcreteSegment>,
    /// True when every reference token mapped to a concrete location.
    pub resolved: bool,
}

/// Parses one `FieldsV1` trie into owned leaf paths.
pub fn parse_fields_v1(fields: &Value) -> Result<Vec<ParsedPath>, FieldsV1Error> {
    let mut paths = Vec::new();
    let mut segments = Vec::new();
    walk(fields, &mut segments, &mut paths)?;
    Ok(paths)
}

/// Resolves an owned path to the value it points at.
#[must_use]
pub fn value_at<'a>(root: &'a Value, segments: &[ConcreteSegment]) -> Option<&'a Value> {
    let mut current = root;
    for segment in segments {
        current = match segment {
            ConcreteSegment::Field(name) => current.get(name)?,
            ConcreteSegment::Index(index) => current.get(*index)?,
        };
    }
    Some(current)
}

fn walk(
    fields: &Value,
    segments: &mut Vec<PathSegment>,
    paths: &mut Vec<ParsedPath>,
) -> Result<(), FieldsV1Error> {
    let object = fields.as_object().ok_or(FieldsV1Error::NotAnObject)?;
    if object.is_empty() {
        if !segments.is_empty() {
            paths.push(ParsedPath {
                segments: segments.clone(),
            });
        }
        return Ok(());
    }

    for (key, child) in object {
        let child_object = child
            .as_object()
            .ok_or_else(|| FieldsV1Error::EntryNotAnObject { key: key.clone() })?;

        if key == "." {
            if !child_object.is_empty() {
                return Err(FieldsV1Error::SelfEntryNotEmpty { key: key.clone() });
            }
            paths.push(ParsedPath {
                segments: segments.clone(),
            });
            continue;
        }

        let length = segments.len();
        if let Some(name) = key.strip_prefix("f:") {
            if name.is_empty() {
                return Err(FieldsV1Error::EmptyFieldName);
            }
            segments.push(PathSegment::Field(name.to_owned()));
        } else if let Some(raw) = key.strip_prefix("i:") {
            let index = raw
                .parse::<u32>()
                .map_err(|_| FieldsV1Error::InvalidIndex {
                    value: raw.to_owned(),
                })?;
            segments.push(PathSegment::Index(index));
        } else if let Some(raw) = key.strip_prefix("k:") {
            let parsed: Value =
                serde_json::from_str(raw).map_err(|error| FieldsV1Error::InvalidKeyPayload {
                    value: raw.to_owned(),
                    reason: error.to_string(),
                })?;
            let Value::Object(fields) = &parsed else {
                return Err(FieldsV1Error::KeyPayloadNotAnObject);
            };
            if fields.is_empty() {
                return Err(FieldsV1Error::EmptyKeyPayload);
            }
            segments.push(PathSegment::Key(parsed));
        } else if let Some(raw) = key.strip_prefix("v:") {
            let parsed: Value =
                serde_json::from_str(raw).map_err(|error| FieldsV1Error::InvalidValuePayload {
                    value: raw.to_owned(),
                    reason: error.to_string(),
                })?;
            segments.push(PathSegment::Value(parsed));
        } else {
            return Err(FieldsV1Error::UnknownKey { key: key.clone() });
        }
        walk(child, segments, paths)?;
        segments.truncate(length);
    }
    Ok(())
}

fn key_matches(key: &Value, item: &Value) -> bool {
    let (Value::Object(key_fields), Value::Object(item_fields)) = (key, item) else {
        return false;
    };
    key_fields
        .iter()
        .all(|(field, expected)| item_fields.get(field) == Some(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pointers(fields: &Value, root: &Value) -> Vec<String> {
        parse_fields_v1(fields)
            .expect("valid FieldsV1")
            .iter()
            .map(|path| path.resolve(root).pointer.into_string())
            .collect()
    }

    #[test]
    fn leaves_and_self_markers_become_pointers() {
        let fields = json!({
            "f:metadata": {
                "f:annotations": {
                    ".": {},
                    "f:example.com/key": {}
                }
            },
            "f:spec": {
                "f:template": {
                    "f:spec": {
                        "f:containers": {},
                        "f:timeoutSeconds": {}
                    }
                }
            }
        });
        let mut paths = pointers(&fields, &json!({}));
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "/metadata/annotations",
                "/metadata/annotations/example.com~1key",
                "/spec/template/spec/containers",
                "/spec/template/spec/timeoutSeconds",
            ]
        );
    }

    #[test]
    fn list_indexes_and_keyed_lists_resolve_to_indexes() {
        let root = json!({
            "spec": {
                "traffic": [{"percent": 90}, {"percent": 10}],
                "containers": [{"name": "web", "image": "nginx"}, {"name": "sidecar"}]
            }
        });
        let fields = json!({
            "f:spec": {
                "f:traffic": {"i:1": {"f:percent": {}}},
                "f:containers": {
                    "k:{\"name\":\"sidecar\"}": {"f:name": {}}
                }
            }
        });
        let mut paths = pointers(&fields, &root);
        paths.sort();
        assert_eq!(
            paths,
            vec!["/spec/containers/1/name", "/spec/traffic/1/percent"]
        );
    }

    #[test]
    fn set_values_resolve_by_exact_json_value() {
        let root = json!({"spec": {"values": ["alpha", "beta"]}});
        let fields = json!({"f:spec": {"f:values": {"v:\"beta\"": {}}}});
        assert_eq!(pointers(&fields, &root), vec!["/spec/values/1"]);
    }

    #[test]
    fn unresolved_keys_keep_a_best_effort_pointer() {
        let parsed = parse_fields_v1(&json!({
            "f:spec": {"f:containers": {"k:{\"name\":\"ghost\"}": {"f:image": {}}}}
        }))
        .expect("valid trie");
        let resolved = parsed[0].resolve(&json!({"spec": {"containers": []}}));
        assert!(!resolved.resolved);
        assert!(resolved.segments.is_empty());
        assert_eq!(
            resolved.pointer.as_str(),
            "/spec/containers/{\"name\":\"ghost\"}/image"
        );
    }

    #[test]
    fn malformed_tries_return_typed_errors() {
        assert_eq!(
            parse_fields_v1(&json!([])).expect_err("not an object"),
            FieldsV1Error::NotAnObject
        );
        assert!(matches!(
            parse_fields_v1(&json!({"f:spec": []})),
            Err(FieldsV1Error::EntryNotAnObject { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({".": {"f:spec": {}}})),
            Err(FieldsV1Error::SelfEntryNotEmpty { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"x:spec": {}})),
            Err(FieldsV1Error::UnknownKey { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"f:": {}})),
            Err(FieldsV1Error::EmptyFieldName)
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"i:first": {}})),
            Err(FieldsV1Error::InvalidIndex { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"k:{": {}})),
            Err(FieldsV1Error::InvalidKeyPayload { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"v:": {}})),
            Err(FieldsV1Error::InvalidValuePayload { .. })
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"k:\"key\"": {}})),
            Err(FieldsV1Error::KeyPayloadNotAnObject)
        ));
        assert!(matches!(
            parse_fields_v1(&json!({"k:{}": {}})),
            Err(FieldsV1Error::EmptyKeyPayload)
        ));
    }

    #[test]
    fn empty_trie_owns_nothing() {
        assert!(
            parse_fields_v1(&json!({}))
                .expect("empty trie is valid")
                .is_empty()
        );
    }
}
