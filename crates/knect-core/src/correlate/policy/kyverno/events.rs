//! Kyverno policy events mapped to their parent Knative Service.

use knect_proto::ResourceId;
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::json;

use crate::graph::GraphBuilder;
use crate::graph::support::{
    add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge, text_field, u64_field,
    warning,
};
use crate::store::ResourceStore;

use super::super::redact::{is_failure, redact_text, severity_from_result, severity_name};
use super::super::resolve::{parent_kservice, scope_id};
use super::super::support::ids_by_kind;
use super::{PolicyInfo, find_policy};

const POLICY_ANNOTATIONS: &[&str] = &[
    "kyverno.io/policy",
    "kyverno.io/policy-name",
    "policies.kyverno.io/name",
    "knect.dev/policy",
];

pub(super) fn add_event_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    policies: &[PolicyInfo],
) {
    for event_id in ids_by_kind(store, "Event") {
        let Some(event) = store.get(&event_id) else {
            continue;
        };
        let reason = text_field(&event, &["reason"]).unwrap_or_default();
        if !matches!(reason, "PolicyViolation" | "PolicyApplied") {
            continue;
        }
        add_event_edge(store, graph, &event_id, &event, policies, reason);
    }
}

fn add_event_edge(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    event_id: &ResourceId,
    event: &DynamicObject,
    policies: &[PolicyInfo],
    reason: &str,
) {
    let Some(reference) =
        field(event, &["involvedObject"]).or_else(|| field(event, &["regarding"]))
    else {
        return;
    };
    let Some(target_id) = scope_id(store, reference, event_id.namespace.as_deref()) else {
        return;
    };
    let Some(kservice) = parent_kservice(store, &target_id).filter(|id| graph.contains_node(id))
    else {
        return;
    };
    let message = redact_text(
        text_field(event, &["message"])
            .or_else(|| text_field(event, &["note"]))
            .unwrap_or_default(),
    );
    let result = annotation(event, "kyverno.io/result").unwrap_or_else(|| {
        if reason == "PolicyViolation" {
            "fail".to_owned()
        } else {
            "pass".to_owned()
        }
    });
    let severity =
        severity_from_result(&result, annotation(event, "kyverno.io/severity").as_deref());
    let rule = annotation(event, "kyverno.io/rule").unwrap_or_default();
    if reason == "PolicyViolation" || is_failure(&result) {
        let mut violation = warning(
            "policy_violation",
            format!("{reason}: {message}"),
            Some(kservice.clone()),
        );
        violation.severity = severity;
        graph.add_warning(violation);
    }
    let policy_reference = policy_from_event(event).unwrap_or_default();
    if policy_reference.is_empty() {
        return;
    }
    let Some(policy) = find_policy(policies, &policy_reference, event_id.namespace.as_deref())
    else {
        graph.add_warning(warning(
            "missing_policy",
            format!("policy event references unknown policy {policy_reference}"),
            Some(kservice),
        ));
        return;
    };
    let mode = policy.rule_mode(&rule);
    let mut edge = new_edge(mode.edge_kind(), &kservice, &policy.id);
    insert_text_metadata(&mut edge, "event", &event_id.name);
    insert_text_metadata(&mut edge, "reason", reason);
    insert_text_metadata(&mut edge, "message", &message);
    insert_text_metadata(&mut edge, "result", &result);
    insert_text_metadata(&mut edge, "severity", severity_name(severity));
    insert_text_metadata(&mut edge, "rule", &rule);
    insert_text_metadata(&mut edge, "policy", &policy.name);
    insert_text_metadata(&mut edge, "policy_reference", &policy_reference);
    insert_text_metadata(&mut edge, "involved_object", &target_id.to_string());
    if let Some(component) = text_field(event, &["reportingComponent"])
        .or_else(|| text_field(event, &["source", "component"]))
    {
        insert_text_metadata(&mut edge, "reporting_component", component);
    }
    if let Some(count) =
        u64_field(event, &["count"]).or_else(|| u64_field(event, &["series", "count"]))
    {
        insert_metadata(&mut edge, "count", json!(count));
    }
    if let Some(timestamp) = text_field(event, &["lastTimestamp"])
        .or_else(|| text_field(event, &["eventTime"]))
        .or_else(|| text_field(event, &["lastUpdateTime"]))
    {
        insert_text_metadata(&mut edge, "last_timestamp", timestamp);
    }
    let suffix = format!("{}:{reason}:{rule}", event_id.name);
    add_unique_edge(graph, edge, Some(&suffix));
}

fn annotation(object: &DynamicObject, key: &str) -> Option<String> {
    object
        .annotations()
        .get(key)
        .or_else(|| object.labels().get(key))
        .cloned()
}

fn policy_from_event(event: &DynamicObject) -> Option<String> {
    for key in POLICY_ANNOTATIONS {
        if let Some(value) = annotation(event, key)
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    let message = text_field(event, &["message"])
        .or_else(|| text_field(event, &["note"]))
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    let start = lower.find("policy ")? + "policy ".len();
    let rest = &message[start..];
    let token = rest
        .split(|character: char| {
            character.is_whitespace() || matches!(character, ',' | ':' | '\'' | '"')
        })
        .next()?;
    (!token.is_empty()).then(|| token.to_owned())
}
