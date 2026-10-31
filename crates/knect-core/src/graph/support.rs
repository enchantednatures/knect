//! Projection helpers shared by the built-in correlators.
//!
//! These helpers keep node construction, timestamp selection, redaction, and
//! deterministic edge identity identical across correlation stages.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use knect_proto::graph::{Badge, Edge, EdgeKind, Health, ResourceNode, Severity, Warning};
use knect_proto::{NodeCategory, ResourceId};
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::{Value, json};

use crate::graph::GraphBuilder;
use crate::store::resource_id;

/// Stable edge identity derived from the edge kind and both endpoints.
///
/// The kind uses the protocol's snake_case name and both endpoints use their
/// `Display` form, which includes the namespace when present.
#[must_use]
pub fn edge_id(kind: EdgeKind, from: &ResourceId, to: &ResourceId) -> String {
    format!("{}:{}->{}", edge_kind_key(kind), from, to)
}

/// Adds a discriminator to an edge identity when several edges of the same
/// kind connect the same pair, for example two tagged route traffic entries.
#[must_use]
pub fn edge_id_with_suffix(
    kind: EdgeKind,
    from: &ResourceId,
    to: &ResourceId,
    suffix: &str,
) -> String {
    format!("{}#{}", edge_id(kind, from, to), suffix)
}

/// Stable snake_case key for an edge kind.
#[must_use]
pub const fn edge_kind_key(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::RoutesTo => "routes_to",
        EdgeKind::Owns => "owns",
        EdgeKind::Scales => "scales",
        EdgeKind::Runs => "runs",
        EdgeKind::Subscribes => "subscribes",
        EdgeKind::DeliversTo => "delivers_to",
        EdgeKind::DeadLetterSink => "dead_letter_sink",
        EdgeKind::UsesSecret => "uses_secret",
        EdgeKind::UsesConfigMap => "uses_config_map",
        EdgeKind::ConnectsTo => "connects_to",
        EdgeKind::BackedBy => "backed_by",
        EdgeKind::Mounts => "mounts",
        EdgeKind::ProvisionedBy => "provisioned_by",
        EdgeKind::ProtectedBy => "protected_by",
        EdgeKind::MutatedBy => "mutated_by",
        EdgeKind::Manages => "manages",
        EdgeKind::References => "references",
        EdgeKind::Targets => "targets",
        EdgeKind::PublishedBy => "published_by",
        EdgeKind::ConsumesFrom => "consumes_from",
        EdgeKind::Selects => "selects",
        EdgeKind::Unknown => "unknown",
    }
}

/// Fields required to project one resource node.
pub(crate) struct NodeSpec {
    /// UI grouping category.
    pub category: NodeCategory,
    /// Current roll-up health.
    pub health: Health,
    /// One-line description.
    pub summary: String,
    /// UI badges.
    pub badges: Vec<Badge>,
}

/// Projects a `DynamicObject` into a deterministic [`ResourceNode`].
pub(crate) fn resource_node(object: &DynamicObject, spec: NodeSpec) -> ResourceNode {
    let id = resource_id(object);
    let (first_seen, last_seen) = object_times(object);
    ResourceNode {
        name: id.name.clone(),
        namespace: id.namespace.clone(),
        id,
        category: spec.category,
        health: spec.health,
        summary: spec.summary,
        labels: sanitized_map(object.labels()),
        annotations: sanitized_map(object.annotations()),
        badges: spec.badges,
        uid: object.uid(),
        first_seen,
        last_seen,
    }
}

/// Returns the object name when it is present and non-empty.
pub(crate) fn object_name(object: &DynamicObject) -> Option<&str> {
    object
        .metadata
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
}

/// Reads a nested field from the dynamic object body.
pub(crate) fn field<'a>(object: &'a DynamicObject, path: &[&str]) -> Option<&'a Value> {
    let mut current = &object.data;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

/// Reads a nested string field from the dynamic object body.
pub(crate) fn text_field<'a>(object: &'a DynamicObject, path: &[&str]) -> Option<&'a str> {
    field(object, path).and_then(Value::as_str)
}

/// Reads a nested unsigned integer field.
pub(crate) fn u64_field(object: &DynamicObject, path: &[&str]) -> Option<u64> {
    field(object, path).and_then(Value::as_u64)
}

/// Finds a status condition by its `type` field.
pub(crate) fn condition<'a>(object: &'a DynamicObject, type_name: &str) -> Option<&'a Value> {
    field(object, &["status", "conditions"])?
        .as_array()?
        .iter()
        .find(|condition| condition.get("type").and_then(Value::as_str) == Some(type_name))
}

/// Returns one condition field as a string.
pub(crate) fn condition_field<'a>(
    object: &'a DynamicObject,
    type_name: &str,
    field_name: &str,
) -> Option<&'a str> {
    condition(object, type_name)?.get(field_name)?.as_str()
}

/// Returns the condition status (`True`, `False`, `Unknown`).
pub(crate) fn condition_status<'a>(object: &'a DynamicObject, type_name: &str) -> Option<&'a str> {
    condition_field(object, type_name, "status")
}

/// Returns the condition reason.
pub(crate) fn condition_reason<'a>(object: &'a DynamicObject, type_name: &str) -> Option<&'a str> {
    condition_field(object, type_name, "reason")
}

/// Returns the condition message.
pub(crate) fn condition_message<'a>(object: &'a DynamicObject, type_name: &str) -> Option<&'a str> {
    condition_field(object, type_name, "message")
}

/// Maps a `Ready` condition to health; a missing condition uses `missing`.
pub(crate) fn ready_health(object: &DynamicObject, missing: Health) -> Health {
    status_health(condition_status(object, "Ready"), missing)
}

/// Maps a condition status string to health.
pub(crate) fn status_health(status: Option<&str>, missing: Health) -> Health {
    match status {
        Some("True") => Health::Healthy,
        Some("False") => Health::Degraded,
        Some("Unknown") => Health::Progressing,
        Some(_) => Health::Unknown,
        None => missing,
    }
}

/// Returns true when any condition reason or message contains one of the
/// case-insensitive needles.
pub(crate) fn conditions_contain(object: &DynamicObject, needles: &[&str]) -> bool {
    let Some(conditions) = field(object, &["status", "conditions"]).and_then(Value::as_array)
    else {
        return false;
    };
    conditions.iter().any(|condition| {
        ["reason", "message"].iter().any(|key| {
            condition
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    let lower = value.to_ascii_lowercase();
                    needles.iter().any(|needle| lower.contains(needle))
                })
        })
    })
}

/// Latest `lastTransitionTime` across all conditions.
pub(crate) fn latest_condition_time(object: &DynamicObject) -> Option<DateTime<Utc>> {
    field(object, &["status", "conditions"])?
        .as_array()?
        .iter()
        .filter_map(|condition| {
            condition
                .get("lastTransitionTime")
                .and_then(Value::as_str)
                .and_then(parse_timestamp)
        })
        .max()
}

/// First and most recent observation time for an object.
///
/// The creation timestamp is preferred; condition transition times refine the
/// last-seen value. `Utc::now()` is only used for malformed fixtures that carry
/// neither.
pub(crate) fn object_times(object: &DynamicObject) -> (DateTime<Utc>, DateTime<Utc>) {
    let created = object.creation_timestamp().and_then(|time| {
        let seconds = time.0.as_second();
        let nanos = u32::try_from(time.0.subsec_nanosecond()).ok()?;
        DateTime::from_timestamp(seconds, nanos)
    });
    let transition = latest_condition_time(object);
    let first = created.or(transition).unwrap_or_else(Utc::now);
    let last = match (created, transition) {
        (Some(created), Some(transition)) => created.max(transition),
        (Some(created), None) => created,
        (None, Some(transition)) => transition,
        (None, None) => first,
    };
    (first, last)
}

/// Parses an RFC 3339 timestamp without panicking.
pub(crate) fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// Creates a healthy edge with an empty label and metadata.
pub(crate) fn new_edge(kind: EdgeKind, from: &ResourceId, to: &ResourceId) -> Edge {
    Edge {
        id: edge_id(kind, from, to),
        from: from.clone(),
        to: to.clone(),
        kind,
        label: None,
        health: Health::Healthy,
        warnings: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

/// Inserts an edge, adding the supplied discriminator (and a numeric suffix if
/// necessary) only when the base identity is already present.
pub(crate) fn add_unique_edge(
    graph: &mut GraphBuilder,
    mut edge: Edge,
    discriminator: Option<&str>,
) {
    if graph.edge(&edge.id).is_some() {
        let base = edge.id.clone();
        if let Some(discriminator) = discriminator {
            edge.id = format!("{base}#{discriminator}");
        }
        let mut counter = 2_u64;
        while graph.edge(&edge.id).is_some() {
            edge.id = format!("{base}#{counter}");
            counter = counter.saturating_add(1);
        }
    }
    graph.add_edge(edge);
}

/// Creates a medium-severity warning.
pub(crate) fn warning(
    code: &str,
    message: impl Into<String>,
    resource: Option<ResourceId>,
) -> Warning {
    Warning {
        code: code.to_owned(),
        message: message.into(),
        severity: Severity::Medium,
        resource,
    }
}

/// Builds a `ResourceId` from a serialized cross-resource reference.
///
/// Missing names or kinds produce `None`. A missing `apiVersion` is inferred
/// from the kind for the resource families knect correlates.
pub(crate) fn reference_id(
    reference: &Value,
    default_namespace: Option<&str>,
) -> Option<ResourceId> {
    let name = reference
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    let kind = reference
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| !kind.is_empty())?;
    let api_version = reference
        .get("apiVersion")
        .and_then(Value::as_str)
        .map_or_else(|| infer_api_version(kind).to_owned(), ToOwned::to_owned);
    let namespace = reference
        .get("namespace")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| default_namespace.map(ToOwned::to_owned));
    Some(ResourceId::new(api_version, kind, namespace, name))
}

/// Creates a namespaced identity from parts.
pub(crate) fn namespaced(
    api_version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> ResourceId {
    ResourceId::new(api_version, kind, namespace.map(ToOwned::to_owned), name)
}

fn infer_api_version(kind: &str) -> &'static str {
    match kind {
        "Service" | "Route" | "Configuration" | "Revision" => "serving.knative.dev/v1",
        "Broker" | "Trigger" | "EventType" => "eventing.knative.dev/v1",
        "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" => "apps/v1",
        "Pod" | "ConfigMap" | "Secret" | "ServiceAccount" => "v1",
        kind if kind.ends_with("Channel") => "messaging.knative.dev/v1",
        _ => "",
    }
}

fn sanitized_map(source: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    source
        .iter()
        .filter(|(key, _)| key.as_str() != "kubectl.kubernetes.io/last-applied-configuration")
        .map(|(key, value)| {
            let value = if redact_key(key) {
                "<redacted>".to_owned()
            } else {
                value.clone()
            };
            (key.clone(), value)
        })
        .collect()
}

fn redact_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "passwd",
        "token",
        "credential",
        "private-key",
        "private_key",
        "client-key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Convenience for attaching JSON metadata to an edge.
pub(crate) fn insert_metadata(edge: &mut Edge, key: &str, value: Value) {
    edge.metadata.insert(key.to_owned(), value);
}

/// Convenience for attaching a string metadata value.
pub(crate) fn insert_text_metadata(edge: &mut Edge, key: &str, value: &str) {
    insert_metadata(edge, key, json!(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(kind: &str, name: &str) -> ResourceId {
        ResourceId::namespaced("serving.knative.dev/v1", kind, "default", name)
    }

    #[test]
    fn edge_ids_are_stable_and_endpoint_sensitive() {
        let route = id("Route", "hello");
        let revision = id("Revision", "hello-00001");
        let first = edge_id(EdgeKind::RoutesTo, &route, &revision);

        assert_eq!(first, edge_id(EdgeKind::RoutesTo, &route, &revision));
        assert!(first.starts_with("routes_to:"));
        assert!(first.contains(&route.to_string()));
        assert!(first.contains(&revision.to_string()));
        assert_ne!(first, edge_id(EdgeKind::Owns, &route, &revision));
        assert_ne!(first, edge_id(EdgeKind::RoutesTo, &revision, &route));
    }

    #[test]
    fn suffixes_only_change_colliding_edges() {
        let route = id("Route", "hello");
        let revision = id("Revision", "hello-00001");
        let base = edge_id(EdgeKind::RoutesTo, &route, &revision);
        let tagged = edge_id_with_suffix(EdgeKind::RoutesTo, &route, &revision, "canary");
        assert_eq!(tagged, format!("{base}#canary"));
        assert_ne!(tagged, base);
    }
}
