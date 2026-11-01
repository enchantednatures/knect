//! Classify managed-fields managers into protocol kinds and diff panes.

use knect_proto::mutation::{ManagerKind, MutationOperation, MutationSource};

/// The protocol classification for one manager name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagerClassification {
    /// UI category.
    pub kind: ManagerKind,
    /// Three-way diff pane the manager contributes to.
    pub source: MutationSource,
}

impl ManagerClassification {
    const fn new(kind: ManagerKind, source: MutationSource) -> Self {
        Self { kind, source }
    }
}

/// Classifies a `managedFields[].manager` value.
///
/// Classification is case-insensitive. The order matters: policy and mesh
/// engines are checked before the generic controller heuristics, and kubectl
/// before the generic user tooling check.
#[must_use]
pub fn classify_manager(manager: &str) -> ManagerClassification {
    let name = manager.trim().to_ascii_lowercase();
    if name.is_empty() {
        return ManagerClassification::new(ManagerKind::Other, MutationSource::Unknown);
    }
    if name.contains("kyverno") {
        return ManagerClassification::new(ManagerKind::Kyverno, MutationSource::PolicyMutators);
    }
    if name == "istiod"
        || name.starts_with("istio")
        || name.contains("sidecar-injector")
        || name.contains("istio.io")
    {
        return ManagerClassification::new(ManagerKind::Istio, MutationSource::PolicyMutators);
    }
    if name.contains("flagger") {
        return ManagerClassification::new(ManagerKind::Flagger, MutationSource::PolicyMutators);
    }
    if name.contains("kubectl") {
        return ManagerClassification::new(ManagerKind::Kubectl, MutationSource::UserIntent);
    }
    if name.contains("helm") {
        return ManagerClassification::new(ManagerKind::Helm, MutationSource::UserIntent);
    }
    if name == "webhook.serving.knative.dev"
        || name.starts_with("webhook.serving.knative.dev/")
        || name.contains("knative")
        || name == "webhook"
    {
        return ManagerClassification::new(ManagerKind::Knative, MutationSource::KnativeDefaults);
    }
    if name.contains("operator") {
        return ManagerClassification::new(ManagerKind::Operator, MutationSource::KnativeDefaults);
    }
    if name.starts_with("webhook.") || is_controller(&name) {
        return ManagerClassification::new(
            ManagerKind::Controller,
            MutationSource::KnativeDefaults,
        );
    }
    ManagerClassification::new(ManagerKind::Other, MutationSource::Unknown)
}

/// Maps the recorded managed-fields operation to the protocol enum.
#[must_use]
pub fn classify_operation(operation: Option<&str>) -> MutationOperation {
    match operation
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "apply" => MutationOperation::Apply,
        "update" => MutationOperation::Update,
        "create" => MutationOperation::Create,
        "delete" => MutationOperation::Delete,
        _ => MutationOperation::Other,
    }
}

fn is_controller(name: &str) -> bool {
    name.contains("controller")
        || name.contains("scheduler")
        || name.contains("kubelet")
        || name.contains("autoscaler")
        || name.contains("provisioner")
        || name.contains("broker")
        || matches!(name, "manager" | "source" | "webhook.serving.knative.dev")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(manager: &str) -> (ManagerKind, MutationSource) {
        let classification = classify_manager(manager);
        (classification.kind, classification.source)
    }

    #[test]
    fn managers_map_to_kinds_and_panes() {
        assert_eq!(
            class("webhook.serving.knative.dev"),
            (ManagerKind::Knative, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("webhook"),
            (ManagerKind::Knative, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("kyverno"),
            (ManagerKind::Kyverno, MutationSource::PolicyMutators)
        );
        assert_eq!(
            class("kyverno-resource-mutator"),
            (ManagerKind::Kyverno, MutationSource::PolicyMutators)
        );
        assert_eq!(
            class("istiod"),
            (ManagerKind::Istio, MutationSource::PolicyMutators)
        );
        assert_eq!(
            class("istio-sidecar-injector"),
            (ManagerKind::Istio, MutationSource::PolicyMutators)
        );
        assert_eq!(
            class("flagger"),
            (ManagerKind::Flagger, MutationSource::PolicyMutators)
        );
        assert_eq!(
            class("kubectl-client-side-apply"),
            (ManagerKind::Kubectl, MutationSource::UserIntent)
        );
        assert_eq!(
            class("helm"),
            (ManagerKind::Helm, MutationSource::UserIntent)
        );
        assert_eq!(
            class("controller"),
            (ManagerKind::Controller, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("kube-controller-manager"),
            (ManagerKind::Controller, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("kube-scheduler"),
            (ManagerKind::Controller, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("local-path-provisioner"),
            (ManagerKind::Controller, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("mtchannel_broker"),
            (ManagerKind::Controller, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("acme-operator"),
            (ManagerKind::Operator, MutationSource::KnativeDefaults)
        );
        assert_eq!(
            class("something-else"),
            (ManagerKind::Other, MutationSource::Unknown)
        );
        assert_eq!(class("  "), (ManagerKind::Other, MutationSource::Unknown));
    }

    #[test]
    fn operations_are_case_insensitive_and_total() {
        assert_eq!(classify_operation(Some("Apply")), MutationOperation::Apply);
        assert_eq!(
            classify_operation(Some("update")),
            MutationOperation::Update
        );
        assert_eq!(
            classify_operation(Some("Create")),
            MutationOperation::Create
        );
        assert_eq!(
            classify_operation(Some("delete")),
            MutationOperation::Delete
        );
        assert_eq!(classify_operation(Some("patch")), MutationOperation::Other);
        assert_eq!(classify_operation(None), MutationOperation::Other);
    }
}
