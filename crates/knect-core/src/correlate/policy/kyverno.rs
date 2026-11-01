//! policy.kyverno correlation stage.
//!
//! Correlates Kyverno policies, policy reports, cluster policy reports, and
//! PolicyViolation/PolicyApplied events to the owning Knative Service.

use std::collections::BTreeMap;

use knect_proto::graph::{Badge, EdgeKind, Health, Severity};
use knect_proto::{NodeCategory, ResourceId};
use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::{Value, json};

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge,
    ready_health, resource_node, text_field, warning,
};
use crate::store::ResourceStore;

use super::redact::{is_failure, redact_text, severity_from_result, severity_name};
use super::resolve::{parent_kservice, scope_id};
use super::support::{KYVERNO_GROUP, POLICY_REPORT_GROUP, ids_by_group_kind};

mod events;

/// Correlates Kyverno policy state.
#[derive(Debug, Default, Clone, Copy)]
pub struct KyvernoPolicyCorrelator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyMode {
    Validation,
    Mutation,
    Mixed,
}

impl PolicyMode {
    const fn edge_kind(self) -> EdgeKind {
        match self {
            Self::Mutation => EdgeKind::MutatedBy,
            Self::Validation | Self::Mixed => EdgeKind::ProtectedBy,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Validation => "validate",
            Self::Mutation => "mutate",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone)]
struct PolicyInfo {
    id: ResourceId,
    name: String,
    namespace: Option<String>,
    kind: String,
    mode: PolicyMode,
    rules: BTreeMap<String, PolicyMode>,
}

impl PolicyInfo {
    fn rule_mode(&self, rule: &str) -> PolicyMode {
        self.rules.get(rule).copied().unwrap_or(match self.mode {
            PolicyMode::Mixed => PolicyMode::Validation,
            other => other,
        })
    }
}

impl Correlator for KyvernoPolicyCorrelator {
    fn name(&self) -> &'static str {
        "policy.kyverno"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let store = &ctx.store;
        let policies = load_policies(store);
        add_policy_nodes(store, graph, &policies);
        add_report_edges(store, graph, &policies);
        events::add_event_edges(store, graph, &policies);
        Ok(())
    }
}

fn load_policies(store: &ResourceStore) -> Vec<PolicyInfo> {
    let mut ids = ids_by_group_kind(store, KYVERNO_GROUP, "ClusterPolicy");
    ids.extend(ids_by_group_kind(store, KYVERNO_GROUP, "Policy"));
    ids.into_iter()
        .filter_map(|id| {
            let object = store.get(&id)?;
            Some(PolicyInfo {
                rules: policy_rules(&object),
                name: id.name.clone(),
                namespace: id.namespace.clone(),
                kind: id.kind.clone(),
                mode: policy_mode(&object),
                id,
            })
        })
        .collect()
}

fn policy_rules(object: &DynamicObject) -> BTreeMap<String, PolicyMode> {
    field(object, &["spec", "rules"])
        .and_then(Value::as_array)
        .map(|rules| {
            rules
                .iter()
                .filter_map(|rule| {
                    let name = rule.get("name").and_then(Value::as_str)?;
                    let mode = if rule.get("mutate").is_some() {
                        PolicyMode::Mutation
                    } else {
                        PolicyMode::Validation
                    };
                    Some((name.to_owned(), mode))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn policy_mode(object: &DynamicObject) -> PolicyMode {
    let rules = policy_rules(object);
    let mut mutate = false;
    let mut validate = false;
    for mode in rules.values() {
        match mode {
            PolicyMode::Mutation => mutate = true,
            PolicyMode::Validation | PolicyMode::Mixed => validate = true,
        }
    }
    match (mutate, validate) {
        (true, true) => PolicyMode::Mixed,
        (true, false) => PolicyMode::Mutation,
        (false, true) => PolicyMode::Validation,
        (false, false) => PolicyMode::Validation,
    }
}

fn add_policy_nodes(store: &ResourceStore, graph: &mut GraphBuilder, policies: &[PolicyInfo]) {
    for info in policies {
        let Some(object) = store.get(&info.id) else {
            continue;
        };
        let mut badges = vec![Badge {
            label: info.mode.label().to_owned(),
            severity: Severity::Info,
            tooltip: Some(format!("{} policy", info.kind)),
        }];
        if let Some(action) = text_field(&object, &["spec", "validationFailureAction"]) {
            badges.push(Badge {
                label: action.to_ascii_lowercase(),
                severity: if action.eq_ignore_ascii_case("Audit") {
                    Severity::Medium
                } else {
                    Severity::Info
                },
                tooltip: Some(action.to_owned()),
            });
        }
        let mut node = resource_node(
            &object,
            NodeSpec {
                category: NodeCategory::Policy,
                health: ready_health(&object, Health::Healthy),
                summary: format!(
                    "{} {} ({} rules)",
                    info.kind,
                    info.mode.label(),
                    info.rules.len()
                ),
                badges,
            },
        );
        node.annotations.insert(
            "knect.dev/policy-mode".to_owned(),
            info.mode.label().to_owned(),
        );
        graph.add_node(node);
    }
}

fn find_policy<'a>(
    policies: &'a [PolicyInfo],
    reference: &str,
    namespace: Option<&str>,
) -> Option<&'a PolicyInfo> {
    let bare = reference.rsplit('/').next().unwrap_or(reference);
    policies
        .iter()
        .find(|policy| {
            policy.namespace.as_deref() == namespace
                && policy.namespace.is_some()
                && policy.name == bare
        })
        .or_else(|| {
            policies
                .iter()
                .find(|policy| policy.namespace.is_none() && policy.name == bare)
        })
        .or_else(|| policies.iter().find(|policy| policy.name == bare))
}

fn add_report_edges(store: &ResourceStore, graph: &mut GraphBuilder, policies: &[PolicyInfo]) {
    let mut reports = ids_by_group_kind(store, POLICY_REPORT_GROUP, "PolicyReport");
    reports.extend(ids_by_group_kind(
        store,
        POLICY_REPORT_GROUP,
        "ClusterPolicyReport",
    ));
    for report_id in reports {
        let Some(report) = store.get(&report_id) else {
            continue;
        };
        let kservice = report_kservice(store, &report_id, &report);
        let Some(results) = field(&report, &["results"]).and_then(Value::as_array) else {
            continue;
        };
        for (index, result) in results.iter().enumerate() {
            add_report_result(
                graph,
                ReportResult {
                    report_id: &report_id,
                    result,
                    index,
                    kservice: kservice.clone(),
                    policies,
                },
            );
        }
    }
}

struct ReportResult<'a> {
    report_id: &'a ResourceId,
    result: &'a Value,
    index: usize,
    kservice: Option<ResourceId>,
    policies: &'a [PolicyInfo],
}

fn add_report_result(graph: &mut GraphBuilder, input: ReportResult<'_>) {
    let policy_reference = input
        .result
        .get("policy")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if policy_reference.is_empty() {
        return;
    }
    let rule = input
        .result
        .get("rule")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = input
        .result
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = redact_text(
        input
            .result
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let severity = severity_from_result(result, explicit_severity(input.result));
    let Some(policy) = find_policy(
        input.policies,
        policy_reference,
        input.report_id.namespace.as_deref(),
    ) else {
        graph.add_warning(warning(
            "missing_policy",
            format!("policy report references unknown policy {policy_reference}"),
            input.kservice.clone(),
        ));
        return;
    };
    let Some(kservice) = input
        .kservice
        .filter(|kservice| graph.contains_node(kservice))
    else {
        return;
    };
    let mode = policy.rule_mode(rule);
    let mut edge = new_edge(mode.edge_kind(), &kservice, &policy.id);
    insert_text_metadata(&mut edge, "report", &input.report_id.name);
    insert_text_metadata(&mut edge, "report_kind", &input.report_id.kind);
    insert_text_metadata(&mut edge, "policy", &policy.name);
    insert_text_metadata(&mut edge, "policy_reference", policy_reference);
    insert_text_metadata(&mut edge, "rule", rule);
    insert_text_metadata(&mut edge, "result", result);
    insert_text_metadata(&mut edge, "severity", severity_name(severity));
    insert_text_metadata(&mut edge, "message", &message);
    if let Some(source) = input.result.get("source").and_then(Value::as_str) {
        insert_text_metadata(&mut edge, "source", source);
    }
    if let Some(scored) = input.result.get("scored").and_then(Value::as_bool) {
        insert_metadata(&mut edge, "scored", json!(scored));
    }
    insert_metadata(&mut edge, "report_index", json!(input.index));
    let suffix = format!("{}:{}:{}", input.report_id.name, rule, input.index);
    add_unique_edge(graph, edge, Some(&suffix));
    if is_failure(result) {
        let mut violation = warning(
            "policy_violation",
            format!("{policy_reference} {rule}: {message}"),
            Some(kservice),
        );
        violation.severity = severity;
        graph.add_warning(violation);
    }
}

fn explicit_severity(result: &Value) -> Option<&str> {
    result.get("severity").and_then(Value::as_str).or_else(|| {
        result
            .get("properties")
            .and_then(|properties| properties.get("severity"))
            .and_then(Value::as_str)
    })
}

fn report_kservice(
    store: &ResourceStore,
    report_id: &ResourceId,
    report: &DynamicObject,
) -> Option<ResourceId> {
    if let Some(scope) = field(report, &["scope"])
        && let Some(scope_id) = scope_id(store, scope, report_id.namespace.as_deref())
        && let Some(kservice) = parent_kservice(store, &scope_id)
    {
        return Some(kservice);
    }
    report.owner_references().iter().find_map(|owner| {
        let owner_id = ResourceId::new(
            owner.api_version.clone(),
            owner.kind.clone(),
            report_id.namespace.clone(),
            owner.name.clone(),
        );
        parent_kservice(store, &owner_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource::tests::test_object;

    #[test]
    fn policy_rules_distinguish_mutation_and_validation() {
        let mut object = test_object(
            "kyverno.io/v1",
            "ClusterPolicy",
            None,
            "knect-e2e-mutate-labels",
        );
        object.data = serde_json::json!({
            "spec": { "rules": [{ "name": "add-labels", "mutate": { "patchStrategicMerge": {} } }] }
        });
        let rules = policy_rules(&object);
        assert_eq!(rules.get("add-labels"), Some(&PolicyMode::Mutation));
        assert_eq!(policy_mode(&object), PolicyMode::Mutation);
    }

    #[test]
    fn report_references_resolve_by_namespace_or_bare_name() {
        let policies = vec![
            PolicyInfo {
                id: ResourceId::cluster_scoped("kyverno.io/v1", "ClusterPolicy", "require"),
                name: "require".to_owned(),
                namespace: None,
                kind: "ClusterPolicy".to_owned(),
                mode: PolicyMode::Validation,
                rules: BTreeMap::new(),
            },
            PolicyInfo {
                id: ResourceId::namespaced("kyverno.io/v1", "Policy", "team", "require"),
                name: "require".to_owned(),
                namespace: Some("team".to_owned()),
                kind: "Policy".to_owned(),
                mode: PolicyMode::Validation,
                rules: BTreeMap::new(),
            },
        ];
        let namespaced = find_policy(&policies, "team/require", Some("team"));
        assert!(namespaced.is_some_and(|policy| policy.namespace.as_deref() == Some("team")));
        let cluster = find_policy(&policies, "require", Some("other"));
        assert!(cluster.is_some_and(|policy| policy.namespace.is_none()));
    }
}
