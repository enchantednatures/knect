//! Human-readable storage node summaries and health mapping.

use knect_proto::graph::Health;
use kube::api::DynamicObject;

use crate::graph::support::text_field;

pub(crate) fn storage_class_summary(object: &DynamicObject) -> String {
    let provisioner = text_field(object, &["provisioner"]).unwrap_or("unknown provisioner");
    match text_field(object, &["reclaimPolicy"]) {
        Some(policy) if !policy.is_empty() => format!("{provisioner}, {policy}"),
        _ => provisioner.to_owned(),
    }
}

pub(crate) fn volume_summary(object: &DynamicObject) -> String {
    let capacity = text_field(object, &["spec", "capacity", "storage"]).unwrap_or("unknown");
    let class = text_field(object, &["spec", "storageClassName"]).unwrap_or("no class");
    let phase = text_field(object, &["status", "phase"]).unwrap_or("unknown");
    format!("{capacity} {class}, {phase}")
}

pub(crate) fn claim_summary(object: &DynamicObject) -> String {
    let capacity = text_field(object, &["status", "capacity", "storage"])
        .or_else(|| text_field(object, &["spec", "resources", "requests", "storage"]))
        .unwrap_or("unknown");
    let class = text_field(object, &["spec", "storageClassName"]).unwrap_or("no class");
    let modes: Vec<&str> = object
        .data
        .get("spec")
        .and_then(|spec| spec.get("accessModes"))
        .and_then(serde_json::Value::as_array)
        .map(|modes| modes.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    format!("{capacity} {class}, {}", modes.join(","))
}

pub(crate) fn backend_summary(object: &DynamicObject) -> String {
    let phase = text_field(object, &["status", "phase"]).unwrap_or("unknown");
    format!("{}, {phase}", resource_kind(object))
}

pub(crate) fn resource_kind(object: &DynamicObject) -> &str {
    object
        .types
        .as_ref()
        .map_or("CephPool", |types| types.kind.as_str())
}

pub(crate) fn claim_health(phase: Option<&str>) -> Health {
    match phase {
        Some("Bound") => Health::Healthy,
        Some("Pending") => Health::Progressing,
        Some("Lost") => Health::Degraded,
        None => Health::Unknown,
        Some(_) => Health::Unknown,
    }
}

pub(crate) fn volume_health(object: &DynamicObject) -> Health {
    match text_field(object, &["status", "phase"]) {
        Some("Bound") => Health::Healthy,
        Some("Available" | "Released") => Health::Progressing,
        Some("Failed") => Health::Degraded,
        None => Health::Unknown,
        Some(_) => Health::Unknown,
    }
}

pub(crate) fn backend_health(object: &DynamicObject) -> Health {
    match text_field(object, &["status", "phase"]) {
        Some("Ready" | "Created") => Health::Healthy,
        Some("Progressing" | "Updating") => Health::Progressing,
        Some("Failed" | "Error") => Health::Degraded,
        None => Health::Unknown,
        Some(_) => Health::Unknown,
    }
}
