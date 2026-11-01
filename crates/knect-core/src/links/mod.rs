//! Link discovery boundary.
//!
//! [`DiscoveryLinkScanner`] discovers explicit Knative bindings and heuristic
//! connections from workload env/envFrom/volumes, Secret material, ConfigMap
//! keys, core Service/Endpoints names, standard ports, and URI schemes. Raw
//! Secret and ConfigMap values never enter a protocol payload: evidence keeps
//! only a credential-free endpoint or `[REDACTED]`.

pub mod candidate;
pub mod dns;
pub mod ports;
pub mod redact;
pub mod uri;

mod bindings;
mod index;
mod projection;
mod scanner;
mod workloads;

pub use candidate::{LinkCandidate, LinkScan, UnresolvedTarget, link_via_key};
pub use knect_proto::links::{Evidence, EvidenceKind, LinkRecord, LinkVia};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;
use crate::graph::GraphBuilder;

/// Scans sanitized objects for explicit and implicit relationships.
pub trait LinkScanner: Send + Sync {
    /// Returns discovered links; implementations may also add graph edges.
    fn scan(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<Vec<LinkRecord>, CoreError>;
}

/// Production scanner backed by the sanitized resource store.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiscoveryLinkScanner;

impl LinkScanner for DiscoveryLinkScanner {
    fn scan(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<Vec<LinkRecord>, CoreError> {
        let scan = scanner::Scanner::new(ctx).run();
        projection::apply_scan(ctx, graph, scan)
    }
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(scanner: std::sync::Arc<dyn LinkScanner>) -> std::sync::Arc<dyn LinkScanner> {
    scanner
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use knect_proto::ResourceId;

    use crate::kube::sanitize::{Sanitizer, SanitizerConfig};
    use crate::secret::SecretMaterialRegistry;
    use crate::store::ResourceStore;

    const SENTINEL: &str = "hunter2-knect-sentinel";

    #[test]
    fn discovery_scanner_is_object_safe() {
        let scanner: Arc<dyn LinkScanner> = Arc::new(DiscoveryLinkScanner);
        let scanner = as_dyn(scanner);
        assert_eq!(Arc::strong_count(&scanner), 1);
    }

    #[test]
    fn secret_material_never_reaches_a_scan_record() {
        let object = kube::api::DynamicObject {
            types: Some(kube::api::TypeMeta {
                api_version: "v1".to_owned(),
                kind: "Secret".to_owned(),
            }),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("credentials".to_owned()),
                namespace: Some("default".to_owned()),
                ..Default::default()
            },
            data: serde_json::json!({
                "data": {
                    "REDIS_URL": "cmVkaXM6Ly86aHVudGVyMi1rbmVjdC1zZW50aW5lbEByZWRpcy1tYXN0ZXI6NjM3OS8w"
                }
            }),
        };
        let sanitized = Sanitizer::new(SanitizerConfig::default()).sanitize(object);
        let secret_id = ResourceId::namespaced("v1", "Secret", "default", "credentials");
        let registry = SecretMaterialRegistry::new();
        registry.insert(secret_id, sanitized.secret_material);
        let mut store = ResourceStore::new();
        store.upsert(sanitized.object);
        let ctx =
            CorrelationContext::new_with_secrets("fixture", 1, Arc::new(store), Arc::new(registry));
        let mut graph = GraphBuilder::new("fixture");
        let records = DiscoveryLinkScanner
            .scan(&ctx, &mut graph)
            .expect("scan succeeds");
        assert!(records.is_empty());
        let serialized = serde_json::to_string(&graph.build()).expect("topology serializes");
        assert!(!serialized.contains(SENTINEL));
    }
}
