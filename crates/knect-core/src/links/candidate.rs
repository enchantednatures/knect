//! Link candidates and deterministic scan results.
//!
//! A candidate is a resolved source/target pair with redacted evidence. The
//! scan result deduplicates candidates by `(source, target, via)` and records
//! unresolved targets as graph warnings instead of inventing placeholder
//! nodes.

use std::collections::BTreeMap;

use knect_proto::ResourceId;
use knect_proto::graph::{EdgeKind, Health, Severity, Warning};
use knect_proto::links::{Evidence, LinkRecord, LinkVia};

/// Stable snake_case key for a discovery mechanism.
#[must_use]
pub const fn link_via_key(via: LinkVia) -> &'static str {
    match via {
        LinkVia::ExplicitBinding => "explicit_binding",
        LinkVia::Env => "env",
        LinkVia::Secret => "secret",
        LinkVia::ConfigMap => "config_map",
        LinkVia::Dns => "dns",
        LinkVia::OperatorCrd => "operator_crd",
        LinkVia::Storage => "storage",
    }
}

/// A resolved relationship discovered by a scanner.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkCandidate {
    /// Resource that references or depends on the target.
    pub source: ResourceId,
    /// Existing resource being referenced.
    pub target: ResourceId,
    /// Discovery mechanism.
    pub via: LinkVia,
    /// Semantic edge kind to project.
    pub kind: EdgeKind,
    /// Confidence in `0.0..=1.0`.
    pub confidence: f64,
    /// Redacted evidence supporting the link.
    pub evidence: Vec<Evidence>,
}

impl LinkCandidate {
    /// Projects the candidate into a protocol [`LinkRecord`].
    #[must_use]
    pub fn record(&self, health: Health) -> LinkRecord {
        let mut evidence = self.evidence.clone();
        evidence.sort_by(evidence_order);
        evidence.dedup();
        LinkRecord {
            source: self.source.clone(),
            target: self.target.clone(),
            via: self.via,
            confidence: self.confidence.clamp(0.0, 1.0),
            evidence,
            health,
        }
    }
}

/// A reference that did not resolve to a stored object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedTarget {
    /// Referenced name, if the reference carried one.
    pub name: String,
    /// Referenced namespace, if the reference carried one.
    pub namespace: Option<String>,
    /// Credential-free preview of the value that produced the reference.
    pub preview: String,
}

impl UnresolvedTarget {
    /// Creates an unresolved target description.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        namespace: Option<String>,
        preview: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            namespace,
            preview: preview.into(),
        }
    }
}

/// Deterministic result of one link scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkScan {
    candidates: BTreeMap<CandidateKey, LinkCandidate>,
    warnings: Vec<Warning>,
}

impl LinkScan {
    /// Creates an empty scan result.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a candidate, merging evidence for a repeated source/target/via.
    pub fn add(&mut self, candidate: LinkCandidate) {
        let key = CandidateKey::new(&candidate);
        match self.candidates.get_mut(&key) {
            Some(existing) => {
                existing.confidence = existing.confidence.max(candidate.confidence);
                existing.evidence.extend(candidate.evidence);
                existing.evidence.sort_by(evidence_order);
                existing.evidence.dedup();
            }
            None => {
                self.candidates.insert(key, candidate);
            }
        }
    }

    /// Records an unresolved reference as a deduplicated warning.
    pub fn unresolved(&mut self, source: &ResourceId, target: UnresolvedTarget, via: LinkVia) {
        let scope = match &target.namespace {
            Some(namespace) => format!("{namespace}/{}", target.name),
            None => target.name.clone(),
        };
        let warning = Warning {
            code: "link_target_unresolved".to_owned(),
            message: format!(
                "{} {} reference to {scope} ({}) is not in the topology",
                source,
                link_via_key(via),
                target.preview
            ),
            severity: Severity::Medium,
            resource: Some(source.clone()),
        };
        if !self.warnings.iter().any(|existing| {
            existing.code == warning.code
                && existing.message == warning.message
                && existing.resource == warning.resource
        }) {
            self.warnings.push(warning);
        }
    }

    /// Adds a scanner warning, deduplicating byte-identical entries.
    pub fn push_warning(&mut self, warning: Warning) {
        if !self.warnings.iter().any(|existing| {
            existing.code == warning.code
                && existing.message == warning.message
                && existing.resource == warning.resource
        }) {
            self.warnings.push(warning);
        }
    }

    /// Resolved candidates in `(source, target, via)` order.
    pub fn candidates(&self) -> impl Iterator<Item = &LinkCandidate> {
        self.candidates.values()
    }

    /// Number of deduplicated candidates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// True when no candidate resolved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Warnings recorded during the scan, sorted deterministically.
    #[must_use]
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings = self.warnings.clone();
        warnings.sort_by(|left, right| {
            (&left.code, &left.message, &left.resource).cmp(&(
                &right.code,
                &right.message,
                &right.resource,
            ))
        });
        warnings
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    source: ResourceId,
    target: ResourceId,
    via: LinkVia,
}

impl CandidateKey {
    fn new(candidate: &LinkCandidate) -> Self {
        Self {
            source: candidate.source.clone(),
            target: candidate.target.clone(),
            via: candidate.via,
        }
    }
}

fn evidence_order(left: &Evidence, right: &Evidence) -> std::cmp::Ordering {
    (
        left.kind,
        &left.description,
        &left.path,
        &left.value_redacted,
    )
        .cmp(&(
            right.kind,
            &right.description,
            &right.path,
            &right.value_redacted,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use knect_proto::links::EvidenceKind;

    fn id(kind: &str, name: &str) -> ResourceId {
        ResourceId::namespaced("v1", kind, "default", name)
    }

    fn evidence(description: &str) -> Evidence {
        Evidence {
            kind: EvidenceKind::EnvVar,
            description: description.to_owned(),
            path: None,
            value_redacted: Some("[REDACTED]".to_owned()),
        }
    }

    fn candidate(via: LinkVia, confidence: f64, description: &str) -> LinkCandidate {
        LinkCandidate {
            source: id("Revision", "hello-00001"),
            target: id("Service", "redis-master"),
            via,
            kind: EdgeKind::ConnectsTo,
            confidence,
            evidence: vec![evidence(description)],
        }
    }

    #[test]
    fn repeated_candidates_merge_evidence_and_keep_max_confidence() {
        let mut scan = LinkScan::new();
        scan.add(candidate(LinkVia::Env, 0.6, "first"));
        scan.add(candidate(LinkVia::Env, 0.9, "second"));
        scan.add(candidate(LinkVia::Env, 0.9, "second"));
        assert_eq!(scan.len(), 1);
        let only = scan.candidates().next().expect("one candidate");
        assert_eq!(only.confidence, 0.9);
        assert_eq!(only.evidence.len(), 2);
        assert_eq!(only.evidence[0].description, "first");
        assert_eq!(only.evidence[1].description, "second");
    }

    #[test]
    fn different_via_or_target_stays_distinct() {
        let mut scan = LinkScan::new();
        scan.add(candidate(LinkVia::Env, 0.8, "env"));
        scan.add(candidate(LinkVia::Secret, 0.8, "secret"));
        let mut other = candidate(LinkVia::Env, 0.8, "other");
        other.target = id("Service", "postgres-e2e-rw");
        scan.add(other);
        assert_eq!(scan.len(), 3);
    }

    #[test]
    fn records_clamp_confidence_and_sort_evidence() {
        let mut scan = LinkScan::new();
        let mut input = candidate(LinkVia::Env, 4.0, "zeta");
        input.evidence.push(evidence("alpha"));
        input.confidence = -1.0;
        scan.add(input);
        let record = scan
            .candidates()
            .next()
            .expect("candidate")
            .record(Health::Unknown);
        assert!(record.confidence >= 0.0);
        assert!(record.confidence <= 1.0);
        assert_eq!(record.evidence[0].description, "alpha");
        assert_eq!(record.evidence[1].description, "zeta");
        assert_eq!(record.health, Health::Unknown);
    }

    #[test]
    fn unresolved_references_become_deduplicated_warnings() {
        let source = id("Revision", "hello-00001");
        let mut scan = LinkScan::new();
        let target = UnresolvedTarget::new("ghost-db", None, "postgres://ghost-db:5432");
        scan.unresolved(&source, target.clone(), LinkVia::Env);
        scan.unresolved(&source, target, LinkVia::Env);
        let warnings = scan.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "link_target_unresolved");
        assert!(warnings[0].message.contains("ghost-db"));
        assert!(warnings[0].message.contains("postgres://ghost-db:5432"));
    }
}
