#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Fixture-driven link discovery and redaction tests.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use knect_core::correlate::links::LinksCorrelator;
use knect_core::correlate::serving::ServingCorrelator;
use knect_core::correlate::{CorrelationContext, Correlator};
use knect_core::graph::GraphBuilder;
use knect_core::kube::sanitize::{Sanitizer, SanitizerConfig};
use knect_core::links::link_via_key;
use knect_core::secret::SecretMaterialRegistry;
use knect_core::store::{ResourceStore, resource_id};
use knect_proto::graph::{Edge, EdgeKind, Health, Topology};
use knect_proto::links::{EvidenceKind, LinkRecord, LinkVia};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::Value;

const SENTINEL: &str = "hunter2-knect-sentinel";

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/fixtures")
        .join(name)
}

fn load_context() -> CorrelationContext {
    let contents =
        std::fs::read_to_string(fixture_path("links/links.json")).expect("links fixture readable");
    let objects: Vec<DynamicObject> =
        serde_json::from_str(&contents).expect("links fixture is valid JSON");
    let sanitizer = Sanitizer::new(SanitizerConfig::default());
    let mut store = ResourceStore::new();
    let registry = SecretMaterialRegistry::new();
    for object in objects {
        let sanitized = sanitizer.sanitize(object);
        let id = resource_id(&sanitized.object);
        if !sanitized.secret_material.is_empty() {
            registry.insert(id, sanitized.secret_material);
        }
        store.upsert(sanitized.object);
    }
    CorrelationContext::new_with_secrets("fixture", 1, Arc::new(store), Arc::new(registry))
}

fn build_topology(ctx: &CorrelationContext) -> Topology {
    let mut graph = GraphBuilder::new("fixture").with_generation(1);
    ServingCorrelator
        .correlate(ctx, &mut graph)
        .expect("serving correlation succeeds");
    LinksCorrelator
        .correlate(ctx, &mut graph)
        .expect("links correlation succeeds");
    graph.build()
}

fn serving_id(kind: &str, name: &str) -> ResourceId {
    ResourceId::namespaced("serving.knative.dev/v1", kind, "knect-e2e", name)
}

fn core_id(kind: &str, name: &str) -> ResourceId {
    ResourceId::namespaced("v1", kind, "knect-e2e", name)
}

fn revision_id() -> ResourceId {
    serving_id("Revision", "order-processor-00001")
}

fn service_id(name: &str) -> ResourceId {
    core_id("Service", name)
}

fn config_map_id(name: &str) -> ResourceId {
    core_id("ConfigMap", name)
}

fn secret_id(name: &str) -> ResourceId {
    core_id("Secret", name)
}

fn link_record(edge: &Edge) -> LinkRecord {
    let value = edge
        .metadata
        .get("link")
        .expect("link edges carry a full LinkRecord");
    serde_json::from_value(value.clone()).expect("link metadata round-trips")
}

fn link_edge<'a>(
    topology: &'a Topology,
    kind: EdgeKind,
    from: &ResourceId,
    to: &ResourceId,
    via: LinkVia,
) -> &'a Edge {
    topology
        .edges
        .iter()
        .filter(|edge| edge.kind == kind && &edge.from == from && &edge.to == to)
        .find(|edge| {
            edge.metadata
                .get("link")
                .and_then(|value| value.get("via"))
                .and_then(Value::as_str)
                == Some(link_via_key(via))
        })
        .unwrap_or_else(|| panic!("missing {via:?} link {from} -> {to} ({kind:?})"))
}

fn has_link(
    topology: &Topology,
    kind: EdgeKind,
    from: &ResourceId,
    to: &ResourceId,
    via: LinkVia,
) -> bool {
    topology.edges.iter().any(|edge| {
        edge.kind == kind
            && &edge.from == from
            && &edge.to == to
            && edge
                .metadata
                .get("link")
                .and_then(|value| value.get("via"))
                .and_then(Value::as_str)
                == Some(link_via_key(via))
    })
}

fn node<'a>(topology: &'a Topology, name: &str) -> Option<&'a knect_proto::ResourceNode> {
    topology.nodes.iter().find(|node| node.name == name)
}

#[test]
fn explicit_service_binding_links_subject_to_bound_service() {
    let topology = build_topology(&load_context());
    let edge = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &serving_id("Service", "order-processor"),
        LinkVia::ExplicitBinding,
    );
    let record = link_record(edge);
    assert_eq!(record.source, revision_id());
    assert_eq!(record.target, serving_id("Service", "order-processor"));
    assert!(record.confidence >= 1.0);
    assert!(
        record
            .evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::Field)
    );
    assert!(edge.metadata.contains_key("evidence"));
    assert!(edge.label.is_some());
}

#[test]
fn explicit_sink_binding_links_subject_to_sink() {
    let topology = build_topology(&load_context());
    let edge = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &service_id("kafka-bootstrap"),
        LinkVia::ExplicitBinding,
    );
    let record = link_record(edge);
    assert_eq!(record.via, LinkVia::ExplicitBinding);
    assert_eq!(record.target, service_id("kafka-bootstrap"));
    assert!(record.confidence >= 1.0);
    assert!(node(&topology, "kafka-bootstrap").is_some());
}

#[test]
fn redis_url_from_secret_links_to_redis_master_with_redacted_evidence() {
    let topology = build_topology(&load_context());
    let edge = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &service_id("redis-master"),
        LinkVia::Env,
    );
    let record = link_record(edge);
    assert!(record.confidence > 0.0 && record.confidence <= 1.0);
    assert!(record.evidence.iter().any(|evidence| {
        evidence.kind == EvidenceKind::Uri
            && evidence.value_redacted.as_deref() == Some("redis://redis-master.knect-e2e:6379")
    }));
    assert!(record.evidence.iter().any(|evidence| {
        evidence.kind == EvidenceKind::SecretKey
            && evidence
                .description
                .contains("environment variable REDIS_URL")
    }));
    assert!(
        record
            .evidence
            .iter()
            .all(|evidence| !evidence.description.contains(SENTINEL))
    );
    assert_eq!(
        node(&topology, "redis-master").map(|node| node.category),
        Some(NodeCategory::Service)
    );
}

#[test]
fn config_map_value_links_are_redacted() {
    let topology = build_topology(&load_context());
    let uses = link_edge(
        &topology,
        EdgeKind::UsesConfigMap,
        &revision_id(),
        &config_map_id("order-config"),
        LinkVia::ConfigMap,
    );
    let uses_record = link_record(uses);
    assert!(
        uses_record
            .evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::ConfigMapKey)
    );
    assert_eq!(
        node(&topology, "order-config").map(|node| node.category),
        Some(NodeCategory::ConfigMap)
    );

    let connection = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &service_id("postgres-e2e-rw"),
        LinkVia::ConfigMap,
    );
    let record = link_record(connection);
    assert!(record.evidence.iter().any(|evidence| {
        evidence.kind == EvidenceKind::ConfigMapKey
            && evidence.value_redacted.as_deref() == Some("postgres://postgres-e2e-rw:5432")
    }));
    let serialized = serde_json::to_string(&connection.metadata).expect("metadata serializes");
    assert!(!serialized.contains("cm-secret"), "{serialized}");
    assert!(!serialized.contains("orders"), "{serialized}");
}

#[test]
fn uri_dns_and_port_matrix_links_resolve_expected_targets() {
    let topology = build_topology(&load_context());
    let cases = [
        ("mysql-e2e", LinkVia::Env),
        ("rabbitmq", LinkVia::Env),
        ("kafka-bootstrap", LinkVia::Env),
        ("postgres-e2e-rw", LinkVia::Env),
    ];
    for (name, via) in cases {
        assert!(
            has_link(
                &topology,
                EdgeKind::ConnectsTo,
                &revision_id(),
                &service_id(name),
                via
            ),
            "missing {via:?} link to {name}"
        );
    }

    let inventory = ResourceId::namespaced("v1", "Service", "shared", "inventory-service");
    assert!(
        has_link(
            &topology,
            EdgeKind::ConnectsTo,
            &revision_id(),
            &inventory,
            LinkVia::Env
        ),
        "cross-namespace DNS link missing"
    );

    let redis_cluster = ResourceId::namespaced(
        "redis.redis.opstreelabs.in/v1beta2",
        "RedisCluster",
        "knect-e2e",
        "redis-cache",
    );
    let operator_edge = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &redis_cluster,
        LinkVia::Env,
    );
    let operator_record = link_record(operator_edge);
    assert_eq!(operator_record.health, Health::Healthy);
    assert_eq!(
        node(&topology, "redis-cache").map(|node| node.category),
        Some(NodeCategory::BackingService)
    );

    let pod = ResourceId::namespaced("v1", "Pod", "knect-e2e", "order-worker");
    assert!(
        has_link(
            &topology,
            EdgeKind::ConnectsTo,
            &pod,
            &service_id("rabbitmq"),
            LinkVia::Env
        ),
        "Pod env link missing"
    );
}

#[test]
fn external_names_and_free_text_do_not_create_links_or_nodes() {
    let topology = build_topology(&load_context());
    for name in ["api.example.com", "example.com", "localhost", "hello world"] {
        assert!(node(&topology, name).is_none(), "unexpected node {name}");
    }
    for edge in &topology.edges {
        for name in ["api.example.com", "example.com", "localhost"] {
            assert_ne!(edge.to.name, name, "unexpected edge target {name}");
        }
    }
    for warning in &topology.warnings {
        for name in ["api.example.com", "localhost", "hello world"] {
            assert!(
                !warning.message.contains(name),
                "false positive warning: {}",
                warning.message
            );
        }
    }
}

#[test]
fn links_are_deduplicated_and_confidence_is_bounded() {
    let topology = build_topology(&load_context());
    let mut seen: BTreeMap<(ResourceId, ResourceId, LinkVia), usize> = BTreeMap::new();
    for edge in &topology.edges {
        let Some(link) = edge.metadata.get("link") else {
            continue;
        };
        let record: LinkRecord =
            serde_json::from_value(link.clone()).expect("link metadata round-trips");
        assert!(
            (0.0..=1.0).contains(&record.confidence),
            "confidence out of range: {}",
            record.confidence
        );
        let key = (record.source.clone(), record.target.clone(), record.via);
        *seen.entry(key).or_insert(0) += 1;
    }
    for (key, count) in seen {
        assert_eq!(count, 1, "duplicate link for {key:?}");
    }

    let redis_env = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &service_id("redis-master"),
        LinkVia::Env,
    );
    let record = link_record(redis_env);
    let env_evidence = record
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == EvidenceKind::EnvVar)
        .count();
    assert!(
        env_evidence >= 2,
        "expected merged REDIS_PLAIN/REDIS_IP evidence, got {env_evidence}"
    );

    let redis_secret = link_edge(
        &topology,
        EdgeKind::ConnectsTo,
        &revision_id(),
        &service_id("redis-master"),
        LinkVia::Secret,
    );
    assert_eq!(link_record(redis_secret).target, service_id("redis-master"));
}

#[test]
fn links_runs_are_deterministic() {
    let first = serde_json::to_string(&build_topology(&load_context())).expect("topology");
    let second = serde_json::to_string(&build_topology(&load_context())).expect("topology");
    assert_eq!(first, second);
}

#[test]
fn missing_link_target_warns_for_unresolved_links() {
    let topology = build_topology(&load_context());
    assert!(node(&topology, "ghost-db").is_none());
    let warning = topology
        .warnings
        .iter()
        .find(|warning| warning.code == "link_target_unresolved")
        .expect("missing target warning");
    assert!(warning.message.contains("ghost-db"), "{}", warning.message);
    assert!(
        warning.message.contains("postgres://ghost-db:5432"),
        "{}",
        warning.message
    );
}

#[test]
fn secret_sentinel_never_appears_in_links_topology() {
    let topology = build_topology(&load_context());
    let serialized = serde_json::to_string(&topology).expect("topology serializes");
    assert!(
        !serialized.contains(SENTINEL),
        "sentinel leaked into topology"
    );
    assert!(
        serialized.contains("redis://redis-master"),
        "redacted endpoint evidence missing"
    );
    assert!(
        !format!("{topology:?}").contains(SENTINEL),
        "sentinel leaked into Debug output"
    );
    for edge in &topology.edges {
        for value in edge.metadata.values() {
            let metadata = serde_json::to_string(value).expect("metadata serializes");
            assert!(
                !metadata.contains(SENTINEL),
                "sentinel leaked into edge metadata: {metadata}"
            );
        }
        assert!(!format!("{edge:?}").contains(SENTINEL));
    }
    for warning in &topology.warnings {
        assert!(!warning.message.contains(SENTINEL));
        assert!(!format!("{warning:?}").contains(SENTINEL));
    }
    assert!(!serialized.contains("cm-secret"));
    assert!(!serialized.contains("mysql-pass"));
    assert!(!serialized.contains("guest:guest"));
}

#[test]
fn secret_uses_links_are_emitted_for_env_and_volumes() {
    let topology = build_topology(&load_context());
    let env_secret = link_edge(
        &topology,
        EdgeKind::UsesSecret,
        &revision_id(),
        &secret_id("redis-credentials"),
        LinkVia::Secret,
    );
    assert!(
        link_record(env_secret)
            .evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::SecretKey)
    );
    let shared = link_edge(
        &topology,
        EdgeKind::UsesSecret,
        &revision_id(),
        &secret_id("shared-env"),
        LinkVia::Secret,
    );
    assert!(
        link_record(shared)
            .evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::EnvFrom)
    );
}
