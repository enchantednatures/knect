#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use knect_core::config::WatchConfig;
use knect_core::kube::Sanitizer;
use knect_core::kube::sanitize::SanitizerConfig;
use kube::api::{DynamicObject, TypeMeta};

fn object(api_version: &str, kind: &str, name: &str) -> DynamicObject {
    DynamicObject {
        types: Some(TypeMeta {
            api_version: api_version.to_owned(),
            kind: kind.to_owned(),
        }),
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some("default".to_owned()),
            ..Default::default()
        },
        data: serde_json::json!({}),
    }
}

#[test]
fn secret_sentinel_never_reaches_the_serializable_object() {
    let mut secret = object("v1", "Secret", "credentials");
    secret.data = serde_json::json!({
        "data": {"password": "aHVudGVyMi1rbmVjdC1zZW50aW5lbA=="},
        "stringData": {"token": "hunter2-knect-sentinel"},
        "metadata": {"managedFields": [{"manager": "kubectl"}]}
    });
    let sanitized = Sanitizer::from_watch_config(&WatchConfig::default()).sanitize(secret);
    let serialized = serde_json::to_string(&sanitized.object).unwrap();
    assert!(!serialized.contains("hunter2-knect-sentinel"));
    assert!(!serialized.contains("aHVudGVyMi1rbmVjdC1zZW50aW5lbA=="));
    assert!(!serialized.contains("stringData"));
    assert_eq!(
        sanitized
            .secret_material
            .get("data", "password")
            .map(knect_proto::redaction::SecretValue::expose),
        Some("aHVudGVyMi1rbmVjdC1zZW50aW5lbA==")
    );
    assert!(!format!("{:?}", sanitized.secret_material).contains("hunter2-knect-sentinel"));
    assert!(!format!("{sanitized:?}").contains("hunter2-knect-sentinel"));
}

#[test]
fn heavy_annotations_are_stripped_but_preserved_as_evidence() {
    let mut config_map = object("v1", "ConfigMap", "settings");
    config_map.metadata.annotations = Some(BTreeMap::from([
        (
            "kubectl.kubernetes.io/last-applied-configuration".to_owned(),
            "{\"data\":{}}".to_owned(),
        ),
        ("keep.example.com/value".to_owned(), "kept".to_owned()),
    ]));
    let sanitizer = Sanitizer::new(SanitizerConfig {
        strip_annotations: vec!["kubectl.kubernetes.io/last-applied-configuration".to_owned()],
        max_status_bytes: 32 * 1024,
        redact_secrets: true,
    });
    let sanitized = sanitizer.sanitize(config_map);
    assert_eq!(
        sanitized
            .removed_annotations
            .get("kubectl.kubernetes.io/last-applied-configuration")
            .map(String::as_str),
        Some("{\"data\":{}}")
    );
    assert_eq!(
        sanitized
            .object
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get("keep.example.com/value"))
            .map(String::as_str),
        Some("kept")
    );
}

#[test]
fn oversized_status_is_removed_and_reported() {
    let mut pod = object("v1", "Pod", "chatty");
    pod.data = serde_json::json!({
        "status": {"conditions": ["x".repeat(64 * 1024)]},
        "spec": {"containers": []}
    });
    let sanitizer = Sanitizer::new(SanitizerConfig {
        strip_annotations: Vec::new(),
        max_status_bytes: 1024,
        redact_secrets: true,
    });
    let sanitized = sanitizer.sanitize(pod);
    assert!(sanitized.removed_status.is_some());
    assert!(sanitized.object.data.get("status").is_none());
    assert!(sanitized.object.data.get("spec").is_some());
    assert_eq!(sanitized.warnings.len(), 1);
}
