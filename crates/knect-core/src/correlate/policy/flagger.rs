//! policy.flagger correlation stage.
//!
//! Correlates Flagger Canaries to their target workloads, generated Istio
//! resources, metric templates, phase, weight, and health.

use std::collections::BTreeMap;

use knect_proto::graph::{Badge, EdgeKind, Health, Severity};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Value, json};

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge,
    ready_health, reference_id, resource_node, text_field, u64_field, warning,
};
use crate::store::ResourceStore;

use super::support::{FLAGGER_GROUP, SERVING_API, ids_by_group_kind};

mod managed;

/// Correlates Flagger canary rollouts.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlaggerPolicyCorrelator;

type MetricIndex = BTreeMap<(Option<String>, String), ResourceId>;

impl Correlator for FlaggerPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.flagger"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let store = &ctx.store;
        let canaries = ids_by_group_kind(store, FLAGGER_GROUP, "Canary");
        let metrics = metric_index(store);
        for id in &canaries {
            let Some(object) = store.get(id) else {
                continue;
            };
            add_canary_node(graph, id, &object, &metrics);
            add_target_edge(store, graph, id, &object);
        }
        managed::add_managed_edges(store, graph, &canaries);
        Ok(())
    }
}

fn metric_index(store: &ResourceStore) -> MetricIndex {
    ids_by_group_kind(store, FLAGGER_GROUP, "MetricTemplate")
        .into_iter()
        .map(|id| ((id.namespace.clone(), id.name.clone()), id))
        .collect()
}

struct CanarySignals {
    phase: String,
    current_weight: u64,
    max_weight: u64,
    iterations: u64,
    failed_checks: u64,
    metric_names: Vec<String>,
    metric_templates: Vec<String>,
}

fn canary_signals(object: &DynamicObject) -> CanarySignals {
    let current_weight = u64_field(object, &["status", "canaryWeight"])
        .or_else(|| u64_field(object, &["status", "currentWeight"]))
        .unwrap_or(0);
    let max_weight = u64_field(object, &["spec", "analysis", "maxWeight"]).unwrap_or(0);
    let iterations = u64_field(object, &["status", "iterations"]).unwrap_or(0);
    let failed_checks = u64_field(object, &["status", "failedChecks"]).unwrap_or(0);
    let (metric_names, metric_templates) = metric_refs(object);
    let phase = text_field(object, &["status", "phase"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| derived_phase(object));
    CanarySignals {
        phase,
        current_weight,
        max_weight,
        iterations,
        failed_checks,
        metric_names,
        metric_templates,
    }
}

fn metric_refs(object: &DynamicObject) -> (Vec<String>, Vec<String>) {
    let Some(metrics) = field(object, &["spec", "analysis", "metrics"]).and_then(Value::as_array)
    else {
        return (Vec::new(), Vec::new());
    };
    let mut names = Vec::new();
    let mut templates = Vec::new();
    for metric in metrics {
        if let Some(name) = metric.get("name").and_then(Value::as_str) {
            names.push(name.to_owned());
        }
        if let Some(template) = metric
            .get("templateRef")
            .and_then(|reference| reference.get("name"))
            .and_then(Value::as_str)
        {
            templates.push(template.to_owned());
            names.push(template.to_owned());
        }
    }
    names.sort();
    names.dedup();
    templates.sort();
    templates.dedup();
    (names, templates)
}

fn derived_phase(object: &DynamicObject) -> String {
    let Some(conditions) = field(object, &["status", "conditions"]).and_then(Value::as_array)
    else {
        return "Unknown".to_owned();
    };
    for condition in conditions {
        let kind = condition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let status = condition
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match (kind, status) {
            ("Failed", "True") => return "Failed".to_owned(),
            ("Promoting", "True") => return "Progressing".to_owned(),
            ("Promoted", "True") => return "Initialized".to_owned(),
            _ => {}
        }
    }
    "Unknown".to_owned()
}

fn canary_health(signals: &CanarySignals, object: &DynamicObject) -> Health {
    if signals.failed_checks > 0 {
        return Health::Degraded;
    }
    match signals.phase.to_ascii_lowercase().as_str() {
        "failed" | "terminating" => Health::Degraded,
        "progressing" | "promoting" | "waiting" | "finalising" => Health::Progressing,
        "initialized" | "initializing" | "initialising" | "succeeded" => Health::Healthy,
        _ => ready_health(object, Health::Unknown),
    }
}

fn add_canary_node(
    graph: &mut GraphBuilder,
    id: &ResourceId,
    object: &DynamicObject,
    metrics: &MetricIndex,
) {
    let signals = canary_signals(object);
    let health = canary_health(&signals, object);
    let mut summary = format!(
        "{}, {}/{}% weight, {} iterations, {} failed checks",
        signals.phase,
        signals.current_weight,
        signals.max_weight,
        signals.iterations,
        signals.failed_checks
    );
    if !signals.metric_names.is_empty() {
        summary.push_str(&format!(", {} metrics", signals.metric_names.len()));
    }
    let mut badges = vec![Badge {
        label: signals.phase.to_ascii_lowercase().replace(' ', "-"),
        severity: health_severity(health),
        tooltip: None,
    }];
    if signals.current_weight > 0 || signals.max_weight > 0 {
        badges.push(Badge {
            label: "canary-weight".to_owned(),
            severity: Severity::Info,
            tooltip: Some(format!(
                "{}/{}%",
                signals.current_weight, signals.max_weight
            )),
        });
    }
    if signals.failed_checks > 0 {
        badges.push(Badge {
            label: "failed-checks".to_owned(),
            severity: Severity::High,
            tooltip: Some(format!("{} failed checks", signals.failed_checks)),
        });
    }
    if !signals.metric_names.is_empty() {
        badges.push(Badge {
            label: "metrics".to_owned(),
            severity: Severity::Info,
            tooltip: Some(signals.metric_names.join(", ")),
        });
    }
    let mut node = resource_node(
        object,
        NodeSpec {
            category: NodeCategory::Canary,
            health,
            summary,
            badges,
        },
    );
    if !signals.metric_templates.is_empty() {
        node.annotations.insert(
            "knect.dev/metric-templates".to_owned(),
            signals.metric_templates.join(","),
        );
    }
    for template in &signals.metric_templates {
        let key = (id.namespace.clone(), template.clone());
        if !metrics.contains_key(&key) {
            graph.add_warning(warning(
                "missing_metric_template",
                format!(
                    "Canary {} references missing MetricTemplate {template}",
                    id.name
                ),
                Some(id.clone()),
            ));
        }
    }
    graph.add_node(node);
}

fn health_severity(health: Health) -> Severity {
    match health {
        Health::Healthy => Severity::Info,
        Health::Progressing => Severity::Medium,
        Health::Degraded | Health::Blocked => Severity::High,
        Health::Unknown => Severity::Info,
    }
}

fn add_target_edge(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    id: &ResourceId,
    object: &DynamicObject,
) {
    let Some(reference) = field(object, &["spec", "targetRef"]) else {
        return;
    };
    let Some(target) = reference_id(reference, id.namespace.as_deref()) else {
        return;
    };
    let target = resolve_target(store, &target);
    let Some(target) = target.filter(|target| graph.contains_node(target)) else {
        graph.add_warning(warning(
            "missing_canary_target",
            format!("Canary {} target is not in the topology", id.name),
            Some(id.clone()),
        ));
        return;
    };
    let mut edge = new_edge(EdgeKind::Targets, id, &target);
    insert_text_metadata(&mut edge, "target_kind", &target.kind);
    insert_text_metadata(&mut edge, "target_api_version", &target.api_version);
    insert_canary_metadata(&mut edge, object);
    add_unique_edge(graph, edge, None);
}

fn resolve_target(store: &ResourceStore, target: &ResourceId) -> Option<ResourceId> {
    if store.contains(target) {
        return Some(target.clone());
    }
    if target.kind == "Service" {
        let service = ResourceId::new(
            SERVING_API,
            "Service",
            target.namespace.clone(),
            target.name.clone(),
        );
        if store.contains(&service) {
            return Some(service);
        }
        let app_service = ResourceId::new(
            "v1",
            "Service",
            target.namespace.clone(),
            target.name.clone(),
        );
        if store.contains(&app_service) {
            return Some(app_service);
        }
    }
    None
}

fn insert_canary_metadata(edge: &mut knect_proto::graph::Edge, object: &DynamicObject) {
    if let Some(phase) = text_field(object, &["status", "phase"]) {
        insert_text_metadata(edge, "phase", phase);
    }
    for (key, path) in [
        ("current_weight", &["status", "canaryWeight"][..]),
        ("max_weight", &["spec", "analysis", "maxWeight"][..]),
        ("step_weight", &["spec", "analysis", "stepWeight"][..]),
        ("threshold", &["spec", "analysis", "threshold"][..]),
        ("iterations", &["status", "iterations"][..]),
        ("failed_checks", &["status", "failedChecks"][..]),
    ] {
        if let Some(value) = u64_field(object, path) {
            insert_metadata(edge, key, json!(value));
        }
    }
    if let Some(interval) = text_field(object, &["spec", "analysis", "interval"]) {
        insert_text_metadata(edge, "interval", interval);
    }
    if let Some(metrics) = field(object, &["spec", "analysis", "metrics"]) {
        insert_metadata(edge, "metric_checks", metrics.clone());
    }
    if let Some(hosts) = field(object, &["spec", "service", "hosts"]) {
        insert_metadata(edge, "hosts", hosts.clone());
    }
    insert_metadata(edge, "conditions", super::support::conditions_json(object));
    let (names, templates) = metric_refs(object);
    if !names.is_empty() {
        insert_metadata(edge, "metrics", json!(names));
    }
    if !templates.is_empty() {
        insert_metadata(edge, "metric_templates", json!(templates));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource::tests::test_object;

    #[test]
    fn phases_map_to_health() {
        let signals = CanarySignals {
            phase: "Progressing".to_owned(),
            current_weight: 25,
            max_weight: 50,
            iterations: 2,
            failed_checks: 0,
            metric_names: Vec::new(),
            metric_templates: Vec::new(),
        };
        let object = test_object("flagger.app/v1beta1", "Canary", Some("default"), "order");
        assert_eq!(canary_health(&signals, &object), Health::Progressing);

        let failed = CanarySignals {
            phase: "Failed".to_owned(),
            failed_checks: 2,
            ..signals
        };
        assert_eq!(canary_health(&failed, &object), Health::Degraded);
    }

    #[test]
    fn derived_phase_uses_conditions() {
        let mut object = test_object("flagger.app/v1beta1", "Canary", Some("default"), "order");
        object.data = serde_json::json!({
            "status": { "conditions": [{ "type": "Promoting", "status": "True" }] }
        });
        assert_eq!(derived_phase(&object), "Progressing");
    }
}
