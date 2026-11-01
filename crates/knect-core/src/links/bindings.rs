//! Explicit Knative `ServiceBinding` and `SinkBinding` scanning.

use knect_proto::FieldPath;
use knect_proto::ResourceId;
use knect_proto::graph::{EdgeKind, Severity, Warning};
use knect_proto::links::{Evidence, EvidenceKind, LinkVia};
use knect_proto::redaction::REDACTED;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::graph::support::field;
use crate::links::candidate::LinkCandidate;
use crate::links::index::api_group;
use crate::links::scanner::Scanner;

impl Scanner<'_> {
    /// Scans explicit Knative binding objects.
    pub(super) fn scan_bindings(&mut self) {
        let bindings: Vec<ResourceId> = self
            .ctx
            .store
            .iter()
            .filter(|(id, _)| {
                matches!(
                    (api_group(&id.api_version), id.kind.as_str()),
                    ("serving.knative.dev", "ServiceBinding")
                        | ("sources.knative.dev", "SinkBinding")
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in bindings {
            let Some(object) = self.ctx.store.get(&id) else {
                continue;
            };
            if id.kind == "ServiceBinding" {
                self.scan_service_binding(&id, &object);
            } else {
                self.scan_sink_binding(&id, &object);
            }
        }
    }

    fn scan_service_binding(&mut self, binding_id: &ResourceId, binding: &DynamicObject) {
        let Some(subject_ref) = binding_reference(binding, &["spec", "subject"]) else {
            self.missing_subject(binding_id, "ServiceBinding");
            return;
        };
        let Some(subject) = self.resolve_reference(subject_ref, binding_id.namespace.as_deref())
        else {
            self.unresolved_reference(binding_id, subject_ref, LinkVia::ExplicitBinding);
            return;
        };

        if let Some(service_ref) = binding_reference(binding, &["spec", "service"]) {
            match self.resolve_reference(service_ref, binding_id.namespace.as_deref()) {
                Some(target) => {
                    let description = format!(
                        "ServiceBinding {} binds subject {} to {}",
                        binding_id.name, subject.name, target.name
                    );
                    self.add_binding_link(&subject, &target, &description, "/spec/service");
                }
                None => {
                    self.unresolved_reference(&subject, service_ref, LinkVia::ExplicitBinding);
                }
            }
        }

        let Some(env) = field(binding, &["spec", "env"]).and_then(Value::as_array) else {
            return;
        };
        for (index, entry) in env.iter().enumerate() {
            let path = format!("/spec/env/{index}");
            self.scan_env_var(&subject, entry, &path);
        }
    }

    fn scan_sink_binding(&mut self, binding_id: &ResourceId, binding: &DynamicObject) {
        let Some(subject_ref) = binding_reference(binding, &["spec", "subject"]) else {
            self.missing_subject(binding_id, "SinkBinding");
            return;
        };
        let Some(subject) = self.resolve_reference(subject_ref, binding_id.namespace.as_deref())
        else {
            self.unresolved_reference(binding_id, subject_ref, LinkVia::ExplicitBinding);
            return;
        };

        if let Some(sink_ref) = binding_reference(binding, &["spec", "sink"]) {
            match self.resolve_reference(sink_ref, binding_id.namespace.as_deref()) {
                Some(target) => {
                    let description = format!(
                        "SinkBinding {} delivers subject {} to {}",
                        binding_id.name, subject.name, target.name
                    );
                    self.add_binding_link(&subject, &target, &description, "/spec/sink/ref");
                }
                None => {
                    self.unresolved_reference(&subject, sink_ref, LinkVia::ExplicitBinding);
                }
            }
            return;
        }

        let uri = field(binding, &["spec", "sink", "uri"])
            .and_then(Value::as_str)
            .or_else(|| field(binding, &["status", "sinkUri"]).and_then(Value::as_str));
        if let Some(uri) = uri {
            let description = format!("SinkBinding {} sink", binding_id.name);
            self.add_connection(
                &subject,
                uri,
                LinkVia::ExplicitBinding,
                EvidenceKind::Field,
                &description,
                "/spec/sink/uri",
            );
        }
    }

    fn add_binding_link(
        &mut self,
        source: &ResourceId,
        target: &ResourceId,
        description: &str,
        path: &str,
    ) {
        let evidence = Evidence {
            kind: EvidenceKind::Field,
            description: description.to_owned(),
            path: Some(FieldPath::new(path)),
            value_redacted: Some(REDACTED.to_owned()),
        };
        self.scan.add(LinkCandidate {
            source: source.clone(),
            target: target.clone(),
            via: LinkVia::ExplicitBinding,
            kind: EdgeKind::ConnectsTo,
            confidence: 1.0,
            evidence: vec![evidence],
        });
    }

    fn missing_subject(&mut self, binding_id: &ResourceId, kind: &str) {
        self.scan.push_warning(Warning {
            code: "link_subject_unresolved".to_owned(),
            message: format!("{kind} {} has no resolvable subject", binding_id.name),
            severity: Severity::Medium,
            resource: Some(binding_id.clone()),
        });
    }

    fn unresolved_reference(&mut self, source: &ResourceId, reference: &Value, via: LinkVia) {
        let name = reference
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)");
        let namespace = reference
            .get("namespace")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let kind = reference
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("object");
        let preview = format!("{kind}/{name}");
        self.unresolved_named(source, name, namespace, &preview, via);
    }
}

fn binding_reference<'a>(object: &'a DynamicObject, path: &[&str]) -> Option<&'a Value> {
    let mut nested = path.to_vec();
    nested.push("ref");
    let reference = field(object, &nested).or_else(|| field(object, path))?;
    reference.get("name").and_then(Value::as_str)?;
    Some(reference)
}
