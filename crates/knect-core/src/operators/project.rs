//! Shared projection helpers for backing operator CRDs.
//!
//! Operator CRDs expose readiness, capacity, and condition fields under
//! slightly different names. These helpers keep extraction tolerant while the
//! badge and summary text stays deterministic across every family.

use knect_proto::graph::{Badge, Health, Severity};
use kube::api::DynamicObject;
use serde_json::Value;

use crate::graph::support::{condition_status, field, status_health, u64_field};

use super::OperatorMetadata;

/// Returns the first unsigned integer found at one of the candidate paths.
pub(crate) fn first_u64(object: &DynamicObject, paths: &[&[&str]]) -> Option<u64> {
    paths.iter().find_map(|path| u64_field(object, path))
}

/// Returns the first string found at one of the candidate paths.
pub(crate) fn first_text(object: &DynamicObject, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        field(object, path)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

/// Maps the first present condition from `names` to health.
pub(crate) fn first_condition_health(object: &DynamicObject, names: &[&str]) -> Option<Health> {
    names.iter().find_map(|name| {
        condition_status(object, name).map(|status| status_health(Some(status), Health::Unknown))
    })
}

/// Maps a free-form phase string to health.
pub(crate) fn phase_health(phase: &str) -> Health {
    let lower = phase.to_ascii_lowercase();
    if ["healthy", "ready", "running", "available"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Health::Healthy
    } else if ["fail", "error", "degrad", "unavailable"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Health::Degraded
    } else if ["progress", "creating", "initial", "reconcil", "pending"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Health::Progressing
    } else {
        Health::Unknown
    }
}

/// Infers health from ready and desired replica counts.
pub(crate) fn replica_health(ready: Option<u64>, desired: Option<u64>) -> Health {
    match (ready, desired) {
        (Some(_ready), Some(0)) => Health::Healthy,
        (Some(ready), Some(desired)) if ready >= desired => Health::Healthy,
        (Some(0), Some(desired)) if desired > 0 => Health::Degraded,
        (Some(ready), Some(desired)) if ready < desired => Health::Progressing,
        (Some(_ready), None) => Health::Healthy,
        _ => Health::Unknown,
    }
}

/// Display label for a health state.
pub(crate) const fn health_label(health: Health) -> &'static str {
    match health {
        Health::Healthy => "ready",
        Health::Degraded => "degraded",
        Health::Progressing => "progressing",
        Health::Blocked => "blocked",
        Health::Unknown => "unknown",
    }
}

/// Primary health badge.
pub(crate) fn health_badge(health: Health) -> Badge {
    let severity = match health {
        Health::Healthy => Severity::Info,
        Health::Degraded => Severity::High,
        Health::Progressing => Severity::Medium,
        Health::Blocked => Severity::Critical,
        Health::Unknown => Severity::Low,
    };
    Badge {
        label: health_label(health).to_owned(),
        severity,
        tooltip: None,
    }
}

/// Replica or instance badge, using the family's unit noun.
pub(crate) fn replica_badge(ready: Option<u64>, desired: Option<u64>, unit: &str) -> Option<Badge> {
    let (label, severity) = match (ready, desired) {
        (Some(ready), Some(desired)) if desired > 0 => (
            format!("{ready}/{desired} {unit}"),
            if ready < desired {
                Severity::Medium
            } else {
                Severity::Info
            },
        ),
        (None, Some(desired)) if desired > 0 => (format!("{desired} {unit}"), Severity::Info),
        (Some(ready), _) if ready > 0 => (format!("{ready} {unit}"), Severity::Info),
        _ => return None,
    };
    Some(Badge {
        label,
        severity,
        tooltip: None,
    })
}

/// Partition badge with an optional replication-factor tooltip.
pub(crate) fn partitions_badge(
    partitions: Option<u64>,
    replication_factor: Option<u64>,
) -> Option<Badge> {
    partitions.map(|partitions| Badge {
        label: format!("{partitions} partitions"),
        severity: Severity::Info,
        tooltip: replication_factor.map(|factor| format!("replication factor {factor}")),
    })
}

/// Queue badge, when the CRD exposes a queue name.
pub(crate) fn queue_badge(queue: Option<&str>) -> Option<Badge> {
    queue.map(|queue| Badge {
        label: format!("queue {queue}"),
        severity: Severity::Info,
        tooltip: None,
    })
}

/// Exchange badge, when the CRD exposes an exchange name.
pub(crate) fn exchange_badge(exchange: Option<&str>) -> Option<Badge> {
    exchange.map(|exchange| Badge {
        label: format!("exchange {exchange}"),
        severity: Severity::Info,
        tooltip: None,
    })
}

/// Builds the standard badge set for an operator node.
pub(crate) fn badges(health: Health, metadata: &OperatorMetadata, unit: &str) -> Vec<Badge> {
    let mut badges = vec![health_badge(health)];
    if let Some(badge) = replica_badge(metadata.ready_replicas, metadata.replicas, unit) {
        badges.push(badge);
    }
    if let Some(badge) = partitions_badge(metadata.partitions, metadata.replication_factor) {
        badges.push(badge);
    }
    if let Some(badge) = queue_badge(metadata.queue.as_deref()) {
        badges.push(badge);
    }
    if let Some(badge) = exchange_badge(metadata.exchange.as_deref()) {
        badges.push(badge);
    }
    badges
}

/// Builds the deterministic one-line summary for an operator node.
pub(crate) fn summary(
    label: &str,
    name: &str,
    metadata: &OperatorMetadata,
    health: Health,
    unit: &str,
) -> String {
    let mut parts = vec![format!("{label} {name}")];
    if let Some(badge) = replica_badge(metadata.ready_replicas, metadata.replicas, unit) {
        parts.push(badge.label);
    }
    if let Some(partitions) = metadata.partitions {
        parts.push(format!("{partitions} partitions"));
    }
    if let Some(factor) = metadata.replication_factor {
        parts.push(format!("RF {factor}"));
    }
    if let Some(queue) = &metadata.queue {
        parts.push(format!("queue {queue}"));
    }
    if let Some(exchange) = &metadata.exchange {
        parts.push(format!("exchange {exchange}"));
    }
    if let Some(phase) = &metadata.phase {
        parts.push(phase.clone());
    }
    parts.push(health_label(health).to_owned());
    parts.join(", ")
}
