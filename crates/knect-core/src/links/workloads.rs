//! Workload env, envFrom, and volume scanning.

use knect_proto::ResourceId;
use knect_proto::links::{EvidenceKind, LinkVia};
use kube::api::DynamicObject;
use serde_json::Value;

use crate::links::index::{pod_spec, pod_spec_prefix, workload_ids};
use crate::links::scanner::{ReferenceStyle, Scanner};

const CONTAINER_FIELDS: [&str; 3] = ["containers", "initContainers", "ephemeralContainers"];

impl Scanner<'_> {
    /// Scans every workload Pod template in deterministic store order.
    pub(super) fn scan_workloads(&mut self) {
        for id in workload_ids(&self.ctx.store) {
            let Some(object) = self.ctx.store.get(&id) else {
                continue;
            };
            self.scan_workload(&id, &object);
        }
    }

    fn scan_workload(&mut self, source: &ResourceId, object: &DynamicObject) {
        let Some(spec) = pod_spec(source, object) else {
            return;
        };
        let prefix = pod_spec_prefix(source);
        for field_name in CONTAINER_FIELDS {
            let Some(containers) = spec.get(field_name).and_then(Value::as_array) else {
                continue;
            };
            for (index, container) in containers.iter().enumerate() {
                let path = format!("{prefix}/{field_name}/{index}");
                self.scan_container(source, container, &path);
            }
        }
        let Some(volumes) = spec.get("volumes").and_then(Value::as_array) else {
            return;
        };
        for (index, volume) in volumes.iter().enumerate() {
            let path = format!("{prefix}/volumes/{index}");
            self.scan_volume(source, volume, &path);
        }
    }

    fn scan_container(&mut self, source: &ResourceId, container: &Value, path: &str) {
        if let Some(env) = container.get("env").and_then(Value::as_array) {
            for (index, entry) in env.iter().enumerate() {
                let entry_path = format!("{path}/env/{index}");
                self.scan_env_var(source, entry, &entry_path);
            }
        }
        if let Some(env_from) = container.get("envFrom").and_then(Value::as_array) {
            for (index, entry) in env_from.iter().enumerate() {
                let entry_path = format!("{path}/envFrom/{index}");
                self.scan_env_from(source, entry, &entry_path);
            }
        }
    }

    pub(super) fn scan_env_var(&mut self, source: &ResourceId, env: &Value, path: &str) {
        let name = env
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)");
        let context = format!("environment variable {name}");
        if let Some(value) = env.get("value").and_then(Value::as_str) {
            self.add_connection(
                source,
                value,
                LinkVia::Env,
                EvidenceKind::EnvVar,
                &context,
                &format!("{path}/value"),
            );
        }
        if let Some(secret_ref) = env.pointer("/valueFrom/secretKeyRef") {
            let Some(secret_name) = secret_ref.get("name").and_then(Value::as_str) else {
                return;
            };
            let key = secret_ref.get("key").and_then(Value::as_str);
            self.link_secret(
                source,
                secret_name,
                key,
                ReferenceStyle::ENV_SECRET,
                &context,
                &format!("{path}/valueFrom/secretKeyRef"),
            );
        }
        if let Some(config_map_ref) = env.pointer("/valueFrom/configMapKeyRef") {
            let Some(config_map_name) = config_map_ref.get("name").and_then(Value::as_str) else {
                return;
            };
            let key = config_map_ref.get("key").and_then(Value::as_str);
            self.link_configmap(
                source,
                config_map_name,
                key,
                ReferenceStyle::ENV_CONFIGMAP,
                &context,
                &format!("{path}/valueFrom/configMapKeyRef"),
            );
        }
    }

    fn scan_env_from(&mut self, source: &ResourceId, entry: &Value, path: &str) {
        if let Some(secret_name) = entry.pointer("/secretRef/name").and_then(Value::as_str) {
            self.link_secret(
                source,
                secret_name,
                None,
                ReferenceStyle::ENV_FROM_SECRET,
                "envFrom",
                &format!("{path}/secretRef"),
            );
        }
        if let Some(config_map_name) = entry.pointer("/configMapRef/name").and_then(Value::as_str) {
            self.link_configmap(
                source,
                config_map_name,
                None,
                ReferenceStyle::ENV_FROM_CONFIGMAP,
                "envFrom",
                &format!("{path}/configMapRef"),
            );
        }
    }

    fn scan_volume(&mut self, source: &ResourceId, volume: &Value, path: &str) {
        let volume_name = volume
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)");
        let context = format!("volume {volume_name}");
        if let Some(secret_name) = volume.pointer("/secret/secretName").and_then(Value::as_str) {
            self.link_secret(
                source,
                secret_name,
                None,
                ReferenceStyle::VOLUME_SECRET,
                &context,
                &format!("{path}/secret/secretName"),
            );
        }
        if let Some(config_map_name) = volume.pointer("/configMap/name").and_then(Value::as_str) {
            self.link_configmap(
                source,
                config_map_name,
                None,
                ReferenceStyle::VOLUME_CONFIGMAP,
                &context,
                &format!("{path}/configMap/name"),
            );
        }
        let Some(sources) = volume
            .pointer("/projected/sources")
            .and_then(Value::as_array)
        else {
            return;
        };
        for (index, projected) in sources.iter().enumerate() {
            if let Some(secret_name) = projected.get("secret").and_then(|value| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|_| value.get("optional").and_then(Value::as_bool) != Some(true))
            }) {
                self.link_secret(
                    source,
                    secret_name,
                    None,
                    ReferenceStyle::VOLUME_SECRET,
                    &context,
                    &format!("{path}/projected/sources/{index}/secret"),
                );
            }
            if let Some(config_map_name) = projected.get("configMap").and_then(|value| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|_| value.get("optional").and_then(Value::as_bool) != Some(true))
            }) {
                self.link_configmap(
                    source,
                    config_map_name,
                    None,
                    ReferenceStyle::VOLUME_CONFIGMAP,
                    &context,
                    &format!("{path}/projected/sources/{index}/configMap"),
                );
            }
        }
    }
}
