//! Manifest validation through a server-side dry-run create.

use std::collections::BTreeMap;

use chrono::Utc;
use knect_proto::ResourceId;
use knect_proto::graph::Warning;
use knect_proto::policy::{PolicyKind, PolicyRecord, PolicyResult, Severity, ValidationResult};
use knect_proto::rpc::ValidateManifestParams;
use kube::api::DynamicObject;
use kube::core::Status;
use serde_json::{Map, Value, json};

use crate::error::CoreError;
use crate::kube::{ClusterReader, DryRunResult};
use crate::store::resource_id;

const ADMISSION_WARNING: &str = "admission_warning";

pub(super) async fn validate_manifest(
    reader: &dyn ClusterReader,
    params: ValidateManifestParams,
) -> Result<ValidationResult, CoreError> {
    let object = parse_manifest(&params)?;
    let resource = resource_id(&object);
    match reader.dry_run_create(object).await {
        Ok(result) => accepted(result, &resource),
        Err(CoreError::Kube(kube::Error::Api(status))) => Ok(rejected(&status, &resource)),
        Err(CoreError::Policy(message)) => Ok(rejected_policy(&message, &resource)),
        Err(error) => Err(error),
    }
}

fn parse_manifest(params: &ValidateManifestParams) -> Result<DynamicObject, CoreError> {
    let manifest = params.manifest.trim();
    if manifest.is_empty() {
        return Err(CoreError::Config("manifest must not be empty".to_owned()));
    }
    let document: Value = serde_yaml::from_str(manifest)
        .map_err(|error| CoreError::Config(format!("manifest is not valid YAML: {error}")))?;
    let Value::Object(mut mapping) = document else {
        return Err(CoreError::Config(
            "manifest must be a YAML mapping".to_owned(),
        ));
    };
    apply_defaults(&mut mapping, params)?;
    serde_json::from_value(Value::Object(mapping)).map_err(|error| {
        CoreError::Config(format!(
            "manifest does not describe a Kubernetes object: {error}"
        ))
    })
}

fn apply_defaults(
    mapping: &mut Map<String, Value>,
    params: &ValidateManifestParams,
) -> Result<(), CoreError> {
    if text_field(mapping, "apiVersion")?.is_empty() {
        return Err(CoreError::Config(
            "manifest must include apiVersion".to_owned(),
        ));
    }
    if text_field(mapping, "kind")?.is_empty() {
        let kind = params
            .default_kind
            .as_deref()
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| {
                CoreError::Config("manifest must include kind or supply default_kind".to_owned())
            })?;
        mapping.insert("kind".to_owned(), Value::String(kind.to_owned()));
    }

    let metadata = mapping
        .entry("metadata".to_owned())
        .or_insert_with(|| json!({}));
    let metadata = metadata
        .as_object_mut()
        .ok_or_else(|| CoreError::Config("manifest metadata must be a mapping".to_owned()))?;
    if text_field(metadata, "name")?.is_empty() {
        return Err(CoreError::Config(
            "manifest must include metadata.name".to_owned(),
        ));
    }
    if text_field(metadata, "namespace")?.is_empty()
        && let Some(namespace) = params
            .namespace
            .as_deref()
            .map(str::trim)
            .filter(|namespace| !namespace.is_empty())
    {
        metadata.insert("namespace".to_owned(), Value::String(namespace.to_owned()));
    }
    Ok(())
}

fn text_field(mapping: &Map<String, Value>, key: &str) -> Result<String, CoreError> {
    match mapping.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.trim().to_owned()),
        Some(_) => Err(CoreError::Config(format!(
            "manifest field '{key}' must be a string"
        ))),
    }
}

fn accepted(result: DryRunResult, resource: &ResourceId) -> Result<ValidationResult, CoreError> {
    let mutated_manifest = serde_yaml::to_string(&result.object)
        .map_err(|error| CoreError::Internal(format!("cannot render mutated manifest: {error}")))?;
    let warnings = result
        .warnings
        .into_iter()
        .map(|message| Warning {
            code: ADMISSION_WARNING.to_owned(),
            message,
            severity: Severity::Info,
            resource: Some(resource.clone()),
        })
        .collect();
    Ok(ValidationResult {
        valid: true,
        mutated_manifest: Some(mutated_manifest),
        violations: Vec::new(),
        warnings,
    })
}

fn rejected(status: &Status, resource: &ResourceId) -> ValidationResult {
    ValidationResult {
        valid: false,
        mutated_manifest: None,
        violations: admission_violations(status, resource),
        warnings: Vec::new(),
    }
}

fn rejected_policy(message: &str, resource: &ResourceId) -> ValidationResult {
    ValidationResult {
        valid: false,
        mutated_manifest: None,
        violations: vec![policy_record(
            "policy".to_owned(),
            "policy",
            message.to_owned(),
            Severity::High,
            policy_kind(message),
            resource,
            BTreeMap::new(),
        )],
        warnings: Vec::new(),
    }
}

fn admission_violations(status: &Status, resource: &ResourceId) -> Vec<PolicyRecord> {
    let severity = if status.is_forbidden() {
        Severity::High
    } else {
        Severity::Medium
    };
    let kind = policy_kind(&status.message);
    let name = if status.reason.trim().is_empty() {
        "admission".to_owned()
    } else {
        status.reason.clone()
    };
    let causes = status
        .details
        .as_ref()
        .map(|details| details.causes.as_slice())
        .unwrap_or_default();

    if causes.is_empty() {
        let mut details = BTreeMap::new();
        details.insert("code".to_owned(), status.code.to_string());
        if !status.reason.is_empty() {
            details.insert("reason".to_owned(), status.reason.clone());
        }
        return vec![policy_record(
            format!("{name}-{}", status.code),
            &name,
            status.message.clone(),
            severity,
            kind,
            resource,
            details,
        )];
    }

    causes
        .iter()
        .enumerate()
        .map(|(index, cause)| {
            let message = if cause.message.trim().is_empty() {
                status.message.clone()
            } else {
                cause.message.clone()
            };
            let mut details = BTreeMap::new();
            details.insert("code".to_owned(), status.code.to_string());
            if !cause.reason.is_empty() {
                details.insert("reason".to_owned(), cause.reason.clone());
            }
            if !cause.field.is_empty() {
                details.insert("field".to_owned(), cause.field.clone());
            }
            policy_record(
                format!("{name}-{index}"),
                &name,
                message,
                severity,
                kind,
                resource,
                details,
            )
        })
        .collect()
}

fn policy_record(
    id: String,
    name: &str,
    message: String,
    severity: Severity,
    kind: PolicyKind,
    resource: &ResourceId,
    details: BTreeMap<String, String>,
) -> PolicyRecord {
    PolicyRecord {
        id,
        name: name.to_owned(),
        namespace: resource.namespace.clone(),
        kind,
        resource: Some(resource.clone()),
        severity,
        result: PolicyResult::Fail,
        message,
        observed_at: Some(Utc::now()),
        details,
    }
}

fn policy_kind(message: &str) -> PolicyKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("kyverno") {
        PolicyKind::Kyverno
    } else if lower.contains("flagger") {
        PolicyKind::Flagger
    } else if lower.contains("istio") {
        PolicyKind::Istio
    } else {
        PolicyKind::Other
    }
}
