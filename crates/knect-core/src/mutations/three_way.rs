//! Three-way mutation attribution: user intent, Knative defaults, and
//! policy/service-mesh mutators.
//!
//! Documents are reconstructed from the paths each manager owns, so a pane
//! never invents values that are not present in the live object. When a
//! `kubectl.kubernetes.io/last-applied-configuration` annotation is available
//! it is used as the authoritative user-intent document.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use knect_proto::mutation::{
    DiffLine, FieldOwnership, ManagerKind, MutationDiff, MutationDiffMode, MutationOperation,
    MutationRecord, MutationSource,
};
use knect_proto::redaction::REDACTED;
use knect_proto::{FieldPath, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Map, Value};
use similar::{DiffTag, TextDiff};

use crate::mutations::MutationError;
use crate::mutations::fields_v1::{self, ConcreteSegment, ResolvedPath};
use crate::mutations::managers::{classify_manager, classify_operation};
use crate::mutations::yaml_index::{YamlIndex, render_yaml};
use crate::store::resource_id;

const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";
const BYTES_PREVIEW: usize = 512;

struct EntryFields {
    manager: String,
    kind: ManagerKind,
    source: MutationSource,
    operation: MutationOperation,
    subresource: Option<String>,
    time: Option<DateTime<Utc>>,
    fields: Vec<ResolvedPath>,
}

#[derive(Debug, Clone)]
struct OwnershipRef {
    pointer: String,
    manager: String,
    source: MutationSource,
}

struct RenderedSide {
    source: MutationSource,
    yaml: String,
    index: YamlIndex,
    ownership: Vec<OwnershipRef>,
    changed: BTreeSet<u32>,
}

/// Builds the attribution diff for one object.
pub fn diff(object: &DynamicObject, mode: MutationDiffMode) -> Result<MutationDiff, MutationError> {
    let resource = resource_id(object);
    let root = serde_json::to_value(object)?;
    let kind = object_kind(object, &root);
    let entries = collect_entries(object, &root)?;
    match mode {
        MutationDiffMode::ThreeWay => three_way(resource, &root, &kind, &entries),
        MutationDiffMode::Managers => managers_view(resource, &root, &kind, &entries),
    }
}

/// Builds one `MutationRecord` per managed-fields entry.
pub fn records(object: &DynamicObject) -> Result<Vec<MutationRecord>, MutationError> {
    let root = serde_json::to_value(object)?;
    let kind = object_kind(object, &root);
    let entries = collect_entries(object, &root)?;
    let mut document = root.clone();
    sanitize_document(&mut document, &kind);
    let yaml = render_yaml(&document)?;
    let index = YamlIndex::build(&yaml)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut paths = Vec::new();
        let mut seen = BTreeSet::new();
        for field in &entry.fields {
            if !field.resolved || !seen.insert(field.pointer.as_str().to_owned()) {
                continue;
            }
            let (yaml_line, yaml_column) = index
                .span(&field.pointer)
                .map_or((None, None), |(line, column)| (Some(line), Some(column)));
            let value_preview = fields_v1::value_at(&root, &field.segments)
                .map(|value| preview(value, &kind, &field.pointer));
            paths.push(FieldOwnership {
                path: field.pointer.clone(),
                yaml_line,
                yaml_column,
                value_preview,
            });
        }
        paths.sort_by(|left, right| left.path.cmp(&right.path));
        records.push(MutationRecord {
            manager: entry.manager,
            manager_kind: entry.kind,
            operation: entry.operation,
            subresource: entry.subresource,
            time: entry.time,
            paths,
        });
    }
    Ok(records)
}

fn three_way(
    resource: ResourceId,
    root: &Value,
    kind: &str,
    entries: &[EntryFields],
) -> Result<MutationDiff, MutationError> {
    let user_document = user_document(root, kind, entries);
    let defaults_document = filtered_document(root, kind, entries, MutationSource::KnativeDefaults);
    let policy_document = filtered_document(root, kind, entries, MutationSource::PolicyMutators);

    let mut user_side = user_document
        .as_ref()
        .map(|document| {
            render_side(
                MutationSource::UserIntent,
                document,
                ownership_refs(entries, Some(MutationSource::UserIntent)),
            )
        })
        .transpose()?;
    let mut defaults_side = defaults_document
        .as_ref()
        .map(|document| {
            render_side(
                MutationSource::KnativeDefaults,
                document,
                ownership_refs(entries, Some(MutationSource::KnativeDefaults)),
            )
        })
        .transpose()?;
    let mut policy_side = policy_document
        .as_ref()
        .map(|document| {
            render_side(
                MutationSource::PolicyMutators,
                document,
                ownership_refs(entries, Some(MutationSource::PolicyMutators)),
            )
        })
        .transpose()?;

    if let (Some(user), Some(defaults)) = (user_side.as_mut(), defaults_side.as_mut()) {
        mark_changed(user, defaults);
    }
    if let (Some(defaults), Some(policy)) = (defaults_side.as_mut(), policy_side.as_mut()) {
        mark_changed(defaults, policy);
    }
    if defaults_side.is_none()
        && let (Some(user), Some(policy)) = (user_side.as_mut(), policy_side.as_mut())
    {
        mark_changed(user, policy);
    }

    let mut lines = Vec::new();
    if let Some(side) = &user_side {
        lines.extend(attributed_lines(side));
    }
    if let Some(side) = &defaults_side {
        lines.extend(attributed_lines(side));
    }
    if let Some(side) = &policy_side {
        lines.extend(attributed_lines(side));
    }

    Ok(MutationDiff {
        resource,
        user_intent: user_side.as_ref().map(|side| side.yaml.clone()),
        knative_defaults: defaults_side.as_ref().map(|side| side.yaml.clone()),
        policy_mutators: policy_side.as_ref().map(|side| side.yaml.clone()),
        lines,
    })
}

fn managers_view(
    resource: ResourceId,
    root: &Value,
    kind: &str,
    entries: &[EntryFields],
) -> Result<MutationDiff, MutationError> {
    let mut document = root.clone();
    sanitize_document(&mut document, kind);
    let side = render_side(
        MutationSource::Unknown,
        &document,
        ownership_refs(entries, None),
    )?;
    let lines = manager_lines(&side);
    Ok(MutationDiff {
        resource,
        user_intent: Some(side.yaml),
        knative_defaults: None,
        policy_mutators: None,
        lines,
    })
}

fn collect_entries(
    object: &DynamicObject,
    root: &Value,
) -> Result<Vec<EntryFields>, MutationError> {
    let Some(managed_fields) = object.metadata.managed_fields.as_ref() else {
        return Err(MutationError::MissingManagedFields);
    };
    if managed_fields.is_empty() {
        return Err(MutationError::MissingManagedFields);
    }

    let mut entries = Vec::with_capacity(managed_fields.len());
    for entry in managed_fields {
        let manager = entry.manager.clone().unwrap_or_default();
        let classification = classify_manager(&manager);
        let mut fields = match entry.fields_v1.as_ref() {
            Some(fields) => fields_v1::parse_fields_v1(&fields.0)
                .map_err(|source| MutationError::FieldsV1 {
                    manager: manager.clone(),
                    source,
                })?
                .iter()
                .map(|path| path.resolve(root))
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        fields.sort_by(|left, right| left.pointer.cmp(&right.pointer));
        fields.dedup_by(|left, right| left.pointer == right.pointer);
        entries.push(EntryFields {
            manager,
            kind: classification.kind,
            source: classification.source,
            operation: classify_operation(entry.operation.as_deref()),
            subresource: entry.subresource.clone().filter(|value| !value.is_empty()),
            time: managed_fields_time(entry.time.as_ref()),
            fields,
        });
    }
    entries.sort_by(|left, right| {
        (&left.manager, &left.subresource, left.operation, &left.time).cmp(&(
            &right.manager,
            &right.subresource,
            right.operation,
            &right.time,
        ))
    });
    Ok(entries)
}

fn managed_fields_time(time: Option<&Time>) -> Option<DateTime<Utc>> {
    let timestamp = time?.0;
    let nanos = u32::try_from(timestamp.subsec_nanosecond()).ok()?;
    DateTime::<Utc>::from_timestamp(timestamp.as_second(), nanos)
}

fn ownership_refs(entries: &[EntryFields], source: Option<MutationSource>) -> Vec<OwnershipRef> {
    let mut refs = Vec::new();
    for entry in entries {
        if source.is_some_and(|wanted| entry.source != wanted) {
            continue;
        }
        for field in &entry.fields {
            if !field.resolved {
                continue;
            }
            refs.push(OwnershipRef {
                pointer: field.pointer.as_str().to_owned(),
                manager: entry.manager.clone(),
                source: entry.source,
            });
        }
    }
    refs.sort_by(|left, right| {
        (&left.pointer, &left.manager).cmp(&(&right.pointer, &right.manager))
    });
    refs
}

fn user_document(root: &Value, kind: &str, entries: &[EntryFields]) -> Option<Value> {
    if let Some(mut document) = last_applied_document(root) {
        sanitize_document(&mut document, kind);
        return Some(document);
    }
    filtered_document(root, kind, entries, MutationSource::UserIntent)
}

fn last_applied_document(root: &Value) -> Option<Value> {
    let text = root
        .get("metadata")?
        .get("annotations")?
        .get(LAST_APPLIED)?
        .as_str()?;
    let document: Value = serde_json::from_str(text).ok()?;
    document.is_object().then_some(document)
}

fn filtered_document(
    root: &Value,
    kind: &str,
    entries: &[EntryFields],
    source: MutationSource,
) -> Option<Value> {
    let mut document = Value::Object(identity_document(root));
    let mut inserted = false;
    for entry in entries.iter().filter(|entry| entry.source == source) {
        for field in &entry.fields {
            if !field.resolved {
                continue;
            }
            let Some(value) = fields_v1::value_at(root, &field.segments) else {
                continue;
            };
            if insert_value(&mut document, &field.segments, value.clone()) {
                inserted = true;
            }
        }
    }
    if !inserted {
        return None;
    }
    sanitize_document(&mut document, kind);
    Some(document)
}

fn identity_document(root: &Value) -> Map<String, Value> {
    let mut document = Map::new();
    for key in ["apiVersion", "kind"] {
        if let Some(value) = root.get(key) {
            document.insert(key.to_owned(), value.clone());
        }
    }
    let mut metadata = Map::new();
    for key in ["name", "namespace"] {
        if let Some(value) = root.get("metadata").and_then(|value| value.get(key)) {
            metadata.insert(key.to_owned(), value.clone());
        }
    }
    if !metadata.is_empty() {
        document.insert("metadata".to_owned(), Value::Object(metadata));
    }
    document
}

fn insert_value(target: &mut Value, segments: &[ConcreteSegment], value: Value) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        *target = value;
        return true;
    };
    match first {
        ConcreteSegment::Field(name) => {
            if !target.is_object() {
                *target = Value::Object(Map::new());
            }
            let Some(object) = target.as_object_mut() else {
                return false;
            };
            if rest.is_empty() {
                object.insert(name.clone(), value);
                return true;
            }
            let child = match object.entry(name.clone()) {
                serde_json::map::Entry::Occupied(entry) => entry.into_mut(),
                serde_json::map::Entry::Vacant(entry) => entry.insert(Value::Object(Map::new())),
            };
            if !child.is_object() && !child.is_array() {
                *child = Value::Object(Map::new());
            }
            insert_value(child, rest, value)
        }
        ConcreteSegment::Index(index) => {
            if !target.is_array() {
                *target = Value::Array(Vec::new());
            }
            let Some(items) = target.as_array_mut() else {
                return false;
            };
            while items.len() <= *index {
                items.push(Value::Null);
            }
            let Some(child) = items.get_mut(*index) else {
                return false;
            };
            if rest.is_empty() {
                *child = value;
                return true;
            }
            if !child.is_object() && !child.is_array() {
                *child = Value::Object(Map::new());
            }
            insert_value(child, rest, value)
        }
    }
}

fn sanitize_document(document: &mut Value, kind: &str) {
    let Some(object) = document.as_object_mut() else {
        return;
    };
    if let Some(Value::Object(metadata)) = object.get_mut("metadata") {
        let _ = metadata.remove("managedFields");
        if let Some(Value::Object(annotations)) = metadata.get_mut("annotations") {
            let _ = annotations.remove(LAST_APPLIED);
        }
    }
    if kind == "Secret" {
        for field in ["data", "stringData"] {
            if let Some(Value::Object(values)) = object.get_mut(field) {
                for value in values.values_mut() {
                    *value = Value::String(REDACTED.to_owned());
                }
            }
        }
    }
}

fn render_side(
    source: MutationSource,
    document: &Value,
    ownership: Vec<OwnershipRef>,
) -> Result<RenderedSide, MutationError> {
    let yaml = render_yaml(document)?;
    let index = YamlIndex::build(&yaml)?;
    Ok(RenderedSide {
        source,
        yaml,
        index,
        ownership,
        changed: BTreeSet::new(),
    })
}

fn mark_changed(left: &mut RenderedSide, right: &mut RenderedSide) {
    let diff = TextDiff::from_lines(left.yaml.as_str(), right.yaml.as_str());
    for operation in diff.ops() {
        if operation.tag() == DiffTag::Equal {
            continue;
        }
        for index in operation.old_range() {
            let _ = left.changed.insert(line_number(index));
        }
        for index in operation.new_range() {
            let _ = right.changed.insert(line_number(index));
        }
    }
}

fn attributed_lines(side: &RenderedSide) -> Vec<DiffLine> {
    let mut lines = Vec::with_capacity(side.index.line_count() as usize);
    for (offset, text) in side.yaml.lines().enumerate() {
        let line_number = line_number(offset);
        let path = most_specific(side.index.paths_on_line(line_number));
        let owner = path
            .and_then(|pointer| owning_field(&side.ownership, pointer))
            .map(|owned| owned.manager.clone());
        lines.push(DiffLine {
            line_number,
            side: side.source,
            owner,
            text: text.to_owned(),
            changed: side.changed.contains(&line_number),
            path: path.map(FieldPath::new),
        });
    }
    lines
}

fn manager_lines(side: &RenderedSide) -> Vec<DiffLine> {
    let mut lines = Vec::with_capacity(side.index.line_count() as usize);
    for (offset, text) in side.yaml.lines().enumerate() {
        let line_number = line_number(offset);
        let path = most_specific(side.index.paths_on_line(line_number));
        let owned = path.and_then(|pointer| owning_field(&side.ownership, pointer));
        lines.push(DiffLine {
            line_number,
            side: owned.map_or(MutationSource::Unknown, |entry| entry.source),
            owner: owned.map(|entry| entry.manager.clone()),
            text: text.to_owned(),
            changed: false,
            path: path.map(FieldPath::new),
        });
    }
    lines
}

fn owning_field<'a>(ownership: &'a [OwnershipRef], path: &str) -> Option<&'a OwnershipRef> {
    ownership
        .iter()
        .filter(|owned| pointer_covers(&owned.pointer, path))
        .max_by(|left, right| {
            pointer_depth(&left.pointer)
                .cmp(&pointer_depth(&right.pointer))
                .then_with(|| left.pointer.cmp(&right.pointer))
                .then_with(|| left.manager.cmp(&right.manager))
        })
}

fn pointer_covers(owned: &str, path: &str) -> bool {
    if owned.is_empty() {
        return true;
    }
    path == owned
        || (path.len() > owned.len()
            && path.starts_with(owned)
            && path.as_bytes().get(owned.len()) == Some(&b'/'))
}

fn pointer_depth(pointer: &str) -> usize {
    pointer.matches('/').count()
}

fn most_specific(paths: &[String]) -> Option<&str> {
    paths
        .iter()
        .max_by(|left, right| {
            pointer_depth(left)
                .cmp(&pointer_depth(right))
                .then_with(|| left.cmp(right))
        })
        .map(String::as_str)
}

fn line_number(index: usize) -> u32 {
    u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX)
}

fn object_kind(object: &DynamicObject, root: &Value) -> String {
    object
        .types
        .as_ref()
        .map(|types| types.kind.clone())
        .or_else(|| {
            root.get("kind")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

fn preview(value: &Value, kind: &str, path: &FieldPath) -> String {
    if is_secret_path(path) || (kind == "Secret" && is_secret_storage(path)) {
        return REDACTED.to_owned();
    }
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    truncate(&text, BYTES_PREVIEW)
}

fn is_secret_storage(path: &FieldPath) -> bool {
    path.as_str() == "/data"
        || path.as_str().starts_with("/data/")
        || path.as_str() == "/stringData"
        || path.as_str().starts_with("/stringData/")
}

fn is_secret_path(path: &FieldPath) -> bool {
    path.segments().any(|segment| {
        let lower = segment.to_ascii_lowercase();
        [
            "password",
            "passwd",
            "token",
            "secret",
            "credential",
            "private-key",
            "private_key",
            "client-key",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    })
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let shortened: String = text.chars().take(limit).collect();
    format!("{shortened}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pointer_coverage_respects_segment_boundaries() {
        assert!(pointer_covers("/spec", "/spec/template"));
        assert!(pointer_covers("/spec/template", "/spec/template"));
        assert!(pointer_covers("", "/anything"));
        assert!(!pointer_covers("/spec", "/specification"));
        assert!(!pointer_covers("/spec/template", "/spec"));
    }

    #[test]
    fn most_specific_prefers_deeper_paths() {
        let paths = vec![
            "/spec/template".to_owned(),
            "/spec/template/spec/containers/0".to_owned(),
            "/spec/template/spec/containers/0/image".to_owned(),
        ];
        assert_eq!(
            most_specific(&paths),
            Some("/spec/template/spec/containers/0/image")
        );
    }

    #[test]
    fn insert_value_rebuilds_maps_and_arrays() {
        let mut document = json!({});
        assert!(insert_value(
            &mut document,
            &[
                ConcreteSegment::Field("spec".to_owned()),
                ConcreteSegment::Field("containers".to_owned()),
                ConcreteSegment::Index(0),
                ConcreteSegment::Field("image".to_owned()),
            ],
            json!("nginx"),
        ));
        assert_eq!(
            document,
            json!({"spec": {"containers": [{"image": "nginx"}]}})
        );
    }

    #[test]
    fn secret_previews_are_redacted() {
        assert_eq!(
            preview(&json!("hunter2"), "Secret", &FieldPath::new("/data/token")),
            REDACTED
        );
        assert_eq!(
            preview(
                &json!({"name": "redis"}),
                "Service",
                &FieldPath::new("/spec/secretKeyRef")
            ),
            REDACTED
        );
        assert_eq!(
            preview(&json!("nginx"), "Service", &FieldPath::new("/spec/image")),
            "nginx"
        );
    }

    #[test]
    fn truncation_is_unicode_safe() {
        assert_eq!(truncate("héllo", 4), "héll...");
        assert_eq!(truncate("short", 10), "short");
    }
}
