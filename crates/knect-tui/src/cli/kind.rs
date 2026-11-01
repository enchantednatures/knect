//! Kubernetes kind aliases used by the one-shot CLI.

use knect_proto::graph::NodeCategory;

/// Kind constraint parsed from a resource argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindSpec {
    /// Canonical Kubernetes kind.
    pub kind: String,
    /// Expected `apiVersion`, when the alias implies one.
    pub api_version: Option<String>,
    /// Expected UI category, when the alias implies one.
    pub category: Option<NodeCategory>,
}

pub(super) fn kind_spec(alias: &str) -> KindSpec {
    let lowered = alias.to_ascii_lowercase();
    let (category, kind, api_version): (Option<NodeCategory>, String, Option<&str>) =
        match lowered.as_str() {
            "ksvc" | "kservice" | "knativeservice" => (
                Some(NodeCategory::KService),
                "Service".to_owned(),
                Some("serving.knative.dev/v1"),
            ),
            "broker" => (
                Some(NodeCategory::Broker),
                "Broker".to_owned(),
                Some("eventing.knative.dev/v1"),
            ),
            "trigger" => (
                Some(NodeCategory::Trigger),
                "Trigger".to_owned(),
                Some("eventing.knative.dev/v1"),
            ),
            "revision" | "rev" => (
                Some(NodeCategory::Revision),
                "Revision".to_owned(),
                Some("serving.knative.dev/v1"),
            ),
            "route" => (
                Some(NodeCategory::Route),
                "Route".to_owned(),
                Some("serving.knative.dev/v1"),
            ),
            "configuration" | "config" => (
                Some(NodeCategory::Configuration),
                "Configuration".to_owned(),
                Some("serving.knative.dev/v1"),
            ),
            "pod" | "po" => (Some(NodeCategory::Pod), "Pod".to_owned(), Some("v1")),
            "deployment" | "deploy" => (
                Some(NodeCategory::Deployment),
                "Deployment".to_owned(),
                Some("apps/v1"),
            ),
            "replicaset" | "rs" => (
                Some(NodeCategory::ReplicaSet),
                "ReplicaSet".to_owned(),
                Some("apps/v1"),
            ),
            "svc" | "service" => (
                Some(NodeCategory::Service),
                "Service".to_owned(),
                Some("v1"),
            ),
            "namespace" | "ns" => (
                Some(NodeCategory::Namespace),
                "Namespace".to_owned(),
                Some("v1"),
            ),
            _ => (None, canonical_kind(alias), None),
        };
    KindSpec {
        kind,
        api_version: api_version.map(ToOwned::to_owned),
        category,
    }
}

pub(super) fn canonical_kind(kind: &str) -> String {
    let mut characters = kind.chars();
    match characters.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
    }
}
