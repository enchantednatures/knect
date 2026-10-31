//! Sanitization before objects enter the resource store.
//!
//! Sanitization removes configured heavy annotations, caps oversized status
//! payloads, and moves Secret values out of the serializable object into an
//! in-memory [`SecretMaterial`] map whose formatting output is redacted.

use std::collections::BTreeMap;
use std::fmt;

use knect_proto::redaction::{REDACTED, SecretValue};
use kube::api::DynamicObject;
use serde_json::Value;

use crate::config::WatchConfig;
use crate::store::resource_id;

/// Default maximum serialized status size retained in memory.
pub const DEFAULT_MAX_STATUS_BYTES: usize = 32 * 1024;

/// Sanitization settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizerConfig {
    /// Annotation keys removed from every object.
    pub strip_annotations: Vec<String>,
    /// Maximum serialized status size; larger statuses are dropped.
    pub max_status_bytes: usize,
    /// Move Secret `data`/`stringData` values into [`SecretMaterial`].
    pub redact_secrets: bool,
}

impl Default for SanitizerConfig {
    fn default() -> Self {
        Self {
            strip_annotations: WatchConfig::default().strip_annotations,
            max_status_bytes: DEFAULT_MAX_STATUS_BYTES,
            redact_secrets: true,
        }
    }
}

impl From<&WatchConfig> for SanitizerConfig {
    fn from(config: &WatchConfig) -> Self {
        Self {
            strip_annotations: config.strip_annotations.clone(),
            ..Self::default()
        }
    }
}

impl From<WatchConfig> for SanitizerConfig {
    fn from(config: WatchConfig) -> Self {
        Self::from(&config)
    }
}

/// A status payload that was removed because it exceeded the configured cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedStatus {
    /// Serialized size in bytes.
    pub bytes: usize,
    /// Configured cap in bytes.
    pub limit: usize,
}

/// Secret values retained only in memory.
///
/// Entries are keyed by `<field>/<key>` (for example `data/password`). The type
/// intentionally implements neither `Serialize` nor `Deserialize`; use
/// [`SecretValue::expose`] inside a narrow parsing scope when a correlator needs
/// an actual value.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretMaterial {
    entries: BTreeMap<String, SecretValue>,
}

impl SecretMaterial {
    /// Records one secret value.
    pub fn insert(&mut self, field: &str, key: &str, value: impl Into<String>) {
        self.entries
            .insert(format!("{field}/{key}"), SecretValue::new(value));
    }

    /// Returns a secret value for an explicit field and key.
    #[must_use]
    pub fn get(&self, field: &str, key: &str) -> Option<&SecretValue> {
        self.entries.get(&format!("{field}/{key}"))
    }

    /// Number of retained values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no values were retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Redacted entry keys in deterministic order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Iterates over `<field>/<key>` entries and their secret values.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &SecretValue)> {
        self.entries
            .iter()
            .map(|(entry, value)| (entry.as_str(), value))
    }

    /// Returns a value by its composite `<field>/<key>` entry name.
    #[must_use]
    pub fn get_entry(&self, entry: &str) -> Option<&SecretValue> {
        self.entries.get(entry)
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretMaterial")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("values", &REDACTED)
            .finish()
    }
}

impl fmt::Display for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretMaterial([REDACTED])")
    }
}

/// An object ready for the store plus the evidence gathered while sanitizing.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedObject {
    /// Sanitized object.
    pub object: DynamicObject,
    /// Removed annotation values keyed by annotation name.
    pub removed_annotations: BTreeMap<String, String>,
    /// Secret values retained only in memory.
    pub secret_material: SecretMaterial,
    /// Status removed for exceeding the configured cap.
    pub removed_status: Option<RemovedStatus>,
    /// Non-fatal sanitization warnings.
    pub warnings: Vec<String>,
}

/// Applies the configured sanitization rules to an object.
#[derive(Debug, Clone)]
pub struct Sanitizer {
    config: SanitizerConfig,
}

impl Sanitizer {
    /// Creates a sanitizer from explicit settings.
    #[must_use]
    pub fn new(config: SanitizerConfig) -> Self {
        Self { config }
    }

    /// Creates a sanitizer that inherits annotation stripping from `watch`.
    #[must_use]
    pub fn from_watch_config(watch: &WatchConfig) -> Self {
        Self::new(SanitizerConfig::from(watch))
    }

    /// Returns the sanitizer configuration.
    #[must_use]
    pub fn config(&self) -> &SanitizerConfig {
        &self.config
    }

    /// Sanitizes one object.
    #[must_use]
    pub fn sanitize(&self, mut object: DynamicObject) -> SanitizedObject {
        let removed_annotations = strip_annotations(&mut object, &self.config);
        let removed_status = cap_status(&mut object, self.config.max_status_bytes);
        let mut warnings = Vec::new();
        if let Some(removed) = &removed_status {
            warnings.push(format!(
                "status of {} removed: {} bytes exceeds {} byte limit",
                resource_id(&object),
                removed.bytes,
                removed.limit
            ));
        }
        let secret_material = if self.config.redact_secrets {
            redact_secret(&mut object)
        } else {
            SecretMaterial::default()
        };
        SanitizedObject {
            object,
            removed_annotations,
            secret_material,
            removed_status,
            warnings,
        }
    }
}

fn strip_annotations(
    object: &mut DynamicObject,
    config: &SanitizerConfig,
) -> BTreeMap<String, String> {
    let mut removed = BTreeMap::new();
    let Some(annotations) = object.metadata.annotations.take() else {
        return removed;
    };
    let mut retained = BTreeMap::new();
    for (key, value) in annotations {
        if config.strip_annotations.iter().any(|strip| strip == &key) {
            removed.insert(key, value);
        } else {
            retained.insert(key, value);
        }
    }
    if !retained.is_empty() {
        object.metadata.annotations = Some(retained);
    }
    removed
}

fn cap_status(object: &mut DynamicObject, max_bytes: usize) -> Option<RemovedStatus> {
    let status = object.data.get("status")?;
    let bytes = serde_json::to_vec(status).map_or(usize::MAX, |serialized| serialized.len());
    if bytes <= max_bytes {
        return None;
    }
    if let Some(root) = object.data.as_object_mut() {
        drop(root.remove("status"));
    }
    Some(RemovedStatus {
        bytes,
        limit: max_bytes,
    })
}

fn redact_secret(object: &mut DynamicObject) -> SecretMaterial {
    let mut material = SecretMaterial::default();
    if !is_secret(object) {
        return material;
    }
    if let Some(Value::Object(values)) = object.data.get_mut("data") {
        for (key, value) in values.iter_mut() {
            let raw = value
                .as_str()
                .map_or_else(|| value.to_string(), ToOwned::to_owned);
            material.insert("data", key, raw);
            *value = Value::String(REDACTED.to_owned());
        }
    }
    if let Some(root) = object.data.as_object_mut()
        && let Some(Value::Object(values)) = root.remove("stringData")
    {
        for (key, value) in values {
            let raw = value
                .as_str()
                .map_or_else(|| value.to_string(), ToOwned::to_owned);
            material.insert("stringData", &key, raw);
        }
    }
    material
}

fn is_secret(object: &DynamicObject) -> bool {
    object
        .types
        .as_ref()
        .is_some_and(|types| types.kind == "Secret")
        || object
            .data
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "Secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::TypeMeta;

    fn object(api_version: &str, kind: &str, name: &str) -> DynamicObject {
        DynamicObject {
            types: Some(TypeMeta {
                api_version: api_version.to_owned(),
                kind: kind.to_owned(),
            }),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some("default".to_owned()),
                ..Default::default()
            },
            data: Value::Object(serde_json::Map::new()),
        }
    }

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizerConfig {
            strip_annotations: vec![
                "kubectl.kubernetes.io/last-applied-configuration".to_owned(),
                "deployment.kubernetes.io/revision".to_owned(),
            ],
            max_status_bytes: 128,
            redact_secrets: true,
        })
    }

    #[test]
    fn configured_annotations_are_removed_and_recorded() {
        let mut input = object("serving.knative.dev/v1", "Service", "hello");
        input.metadata.annotations = Some(BTreeMap::from([
            (
                "kubectl.kubernetes.io/last-applied-configuration".to_owned(),
                "{\"kind\":\"Service\"}".to_owned(),
            ),
            (
                "deployment.kubernetes.io/revision".to_owned(),
                "7".to_owned(),
            ),
            ("keep.me/value".to_owned(), "kept".to_owned()),
        ]));
        let sanitized = sanitizer().sanitize(input);
        assert_eq!(
            sanitized
                .removed_annotations
                .get("deployment.kubernetes.io/revision"),
            Some(&"7".to_owned())
        );
        assert_eq!(
            sanitized
                .object
                .metadata
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get("keep.me/value")),
            Some(&"kept".to_owned())
        );
        assert!(
            !sanitized
                .object
                .metadata
                .annotations
                .as_ref()
                .is_some_and(|annotations| annotations
                    .contains_key("kubectl.kubernetes.io/last-applied-configuration"))
        );
    }

    #[test]
    fn oversized_status_is_removed_with_a_warning() {
        let mut input = object("v1", "Pod", "big");
        input.data = serde_json::json!({
            "status": {"message": "x".repeat(1_000)},
            "spec": {"containers": []}
        });
        let sanitized = sanitizer().sanitize(input);
        assert!(sanitized.removed_status.is_some());
        assert!(sanitized.object.data.get("status").is_none());
        assert!(sanitized.object.data.get("spec").is_some());
        assert_eq!(sanitized.warnings.len(), 1);
        assert!(sanitized.warnings[0].contains("status of"));
    }

    #[test]
    fn normal_status_is_preserved() {
        let mut input = object("v1", "Pod", "small");
        input.data = serde_json::json!({"status": {"phase": "Running"}});
        let sanitized = sanitizer().sanitize(input);
        assert!(sanitized.removed_status.is_none());
        assert_eq!(
            sanitized.object.data["status"]["phase"],
            Value::String("Running".to_owned())
        );
        assert!(sanitized.warnings.is_empty());
    }

    #[test]
    fn secret_values_leave_the_serializable_object() {
        let mut input = object("v1", "Secret", "credentials");
        input.data = serde_json::json!({
            "data": {"password": "aHVudGVyMi1rbmVjdC1zZW50aW5lbA=="},
            "stringData": {"token": "hunter2-knect-sentinel"},
            "status": {"phase": "ok"}
        });
        let sanitized = sanitizer().sanitize(input);
        let serialized = serde_json::to_string(&sanitized.object).expect("object serializes");
        assert!(!serialized.contains("hunter2-knect-sentinel"));
        assert!(!serialized.contains("aHVudGVyMi1rbmVjdC1zZW50aW5lbA=="));
        assert!(!serialized.contains("stringData"));
        assert_eq!(
            sanitized
                .secret_material
                .get("data", "password")
                .map(SecretValue::expose),
            Some("aHVudGVyMi1rbmVjdC1zZW50aW5lbA==")
        );
        assert_eq!(
            sanitized
                .secret_material
                .get("stringData", "token")
                .map(SecretValue::expose),
            Some("hunter2-knect-sentinel")
        );
        let debug = format!("{:?}", sanitized.secret_material);
        assert!(!debug.contains("hunter2-knect-sentinel"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!format!("{sanitized:?}").contains("hunter2-knect-sentinel"));
    }

    #[test]
    fn secret_redaction_can_be_disabled() {
        let mut input = object("v1", "Secret", "credentials");
        input.data = serde_json::json!({"data": {"password": "aHVudGVyMg=="}});
        let sanitizer = Sanitizer::new(SanitizerConfig {
            redact_secrets: false,
            ..SanitizerConfig::default()
        });
        let sanitized = sanitizer.sanitize(input);
        assert!(sanitized.secret_material.is_empty());
        assert_eq!(
            sanitized.object.data["data"]["password"],
            Value::String("aHVudGVyMg==".to_owned())
        );
    }

    #[test]
    fn managed_fields_are_preserved() {
        let mut input = object("v1", "ConfigMap", "settings");
        input.metadata.managed_fields = Some(vec![Default::default()]);
        let sanitized = sanitizer().sanitize(input);
        assert!(sanitized.object.metadata.managed_fields.is_some());
    }

    #[test]
    fn watch_config_seeds_the_sanitizer_defaults() {
        let watch = WatchConfig::default();
        let config = SanitizerConfig::from(&watch);
        assert_eq!(config.strip_annotations, watch.strip_annotations);
        assert_eq!(config.max_status_bytes, DEFAULT_MAX_STATUS_BYTES);
        assert!(config.redact_secrets);
    }
}
