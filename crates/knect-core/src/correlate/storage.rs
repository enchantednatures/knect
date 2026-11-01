//! Persistent storage correlation.
//!
//! Projects the `Revision`/`Deployment`/`Pod` -> PVC -> PV -> StorageClass ->
//! Rook Ceph chain and flags ReadWriteOnce claims mounted by Services whose
//! `autoscaling.knative.dev/maxScale` permits more than one replica.

use std::collections::BTreeSet;

use knect_proto::graph::{Badge, EdgeKind, Health, Severity, Warning};
use knect_proto::{NodeCategory, ResourceId};
use kube::api::DynamicObject;
use serde_json::json;

use crate::correlate::{CorrelationContext, Correlator};
use crate::error::CoreError;
use crate::graph::GraphBuilder;
use crate::graph::support::{
    NodeSpec, add_unique_edge, edge_id, insert_metadata, insert_text_metadata, new_edge,
    object_name, resource_node,
};
use crate::storage::{
    CONFLICT_BADGE, CONFLICT_CODE, StorageTrace, access_mode_label, backend_health,
    backend_summary, claim_health, claim_summary, conflict_reason, storage_class_summary, trace,
    volume_health, volume_summary,
};
use crate::store::ResourceStore;

/// Correlates persistent storage into the graph and runs the RWO linter.
#[derive(Debug, Default, Clone, Copy)]
pub struct StorageCorrelator;

impl Correlator for StorageCorrelator {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn correlate(
        &self,
        ctx: &CorrelationContext,
        graph: &mut GraphBuilder,
    ) -> Result<(), CoreError> {
        let trace = trace(ctx);
        add_storage_nodes(&ctx.store, graph, &trace);
        add_mount_edges(graph, &trace);
        add_backing_edges(graph, &trace);
        apply_conflicts(graph, &trace);
        Ok(())
    }
}

fn add_storage_nodes(store: &ResourceStore, graph: &mut GraphBuilder, trace: &StorageTrace) {
    for id in &trace.storage_classes {
        add_node(
            store,
            graph,
            id,
            NodeCategory::StorageClass,
            Health::Healthy,
            storage_class_summary,
        );
    }
    for id in &trace.persistent_volumes {
        add_node(
            store,
            graph,
            id,
            NodeCategory::PersistentVolume,
            Health::Unknown,
            volume_summary,
        );
    }
    for id in &trace.persistent_volume_claims {
        let health = trace.claims.get(id).map_or(Health::Unknown, |facts| {
            claim_health(facts.phase.as_deref())
        });
        add_node(
            store,
            graph,
            id,
            NodeCategory::PersistentVolumeClaim,
            health,
            claim_summary,
        );
    }
    for id in &trace.ceph_pools {
        add_node(
            store,
            graph,
            id,
            NodeCategory::CephPool,
            Health::Unknown,
            backend_summary,
        );
    }
}

fn add_node(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    id: &ResourceId,
    category: NodeCategory,
    missing_health: Health,
    summary: fn(&DynamicObject) -> String,
) {
    let Some(object) = store.get(id) else {
        return;
    };
    if object_name(&object).is_none() {
        return;
    }
    let health = match category {
        NodeCategory::PersistentVolume => volume_health(&object),
        NodeCategory::CephPool => backend_health(&object),
        _ => missing_health,
    };
    graph.add_node(resource_node(
        &object,
        NodeSpec {
            category,
            health,
            summary: summary(&object),
            badges: Vec::new(),
        },
    ));
}

fn add_mount_edges(graph: &mut GraphBuilder, trace: &StorageTrace) {
    for mount in &trace.mounts {
        let mut edge = new_edge(EdgeKind::Mounts, &mount.workload, &mount.pvc);
        edge.label = Some(mount.volume.clone());
        insert_text_metadata(&mut edge, "volume", &mount.volume);
        if let Some(path) = &mount.mount_path {
            insert_text_metadata(&mut edge, "mount_path", path);
        }
        if let Some(facts) = trace.claims.get(&mount.pvc) {
            let modes: Vec<&str> = facts
                .access_modes
                .iter()
                .map(|mode| access_mode_label(*mode))
                .collect();
            insert_metadata(&mut edge, "access_modes", json!(modes));
        }
        add_unique_edge(graph, edge, Some(&mount.volume));
    }
}

fn add_backing_edges(graph: &mut GraphBuilder, trace: &StorageTrace) {
    for (claim, relation) in &trace.claim_relations {
        if let Some(volume) = &relation.pv {
            let mut edge = new_edge(EdgeKind::BackedBy, claim, volume);
            insert_text_metadata(&mut edge, "volume", &volume.name);
            add_unique_edge(graph, edge, None);
        }
        if relation.pv.is_none()
            && let Some(class) = &relation.storage_class
        {
            let mut edge = new_edge(EdgeKind::ProvisionedBy, claim, class);
            insert_text_metadata(&mut edge, "storage_class", &class.name);
            add_unique_edge(graph, edge, None);
        }
    }
    for (volume, class) in &trace.volume_storage_classes {
        let mut edge = new_edge(EdgeKind::ProvisionedBy, volume, class);
        insert_text_metadata(&mut edge, "storage_class", &class.name);
        add_unique_edge(graph, edge, None);
    }
    for (class, backend) in &trace.class_backends {
        let mut edge = new_edge(EdgeKind::BackedBy, class, backend);
        insert_text_metadata(&mut edge, "backend_kind", &backend.kind);
        insert_text_metadata(&mut edge, "backend_name", &backend.name);
        add_unique_edge(graph, edge, None);
    }
}

fn apply_conflicts(graph: &mut GraphBuilder, trace: &StorageTrace) {
    let mut warned: BTreeSet<(String, ResourceId)> = BTreeSet::new();
    for conflict in &trace.conflicts {
        let message = format!(
            "{} mounts {}: {}",
            conflict.workload.name,
            conflict.pvc.name,
            conflict_reason(conflict.access_mode, conflict.max_scale)
        );
        let warning = Warning {
            code: CONFLICT_CODE.to_owned(),
            message: message.clone(),
            severity: Severity::High,
            resource: Some(conflict.workload.clone()),
        };
        add_edge_warning(
            graph,
            &edge_id(EdgeKind::Mounts, &conflict.workload, &conflict.pvc),
            warning,
            &conflict.access_mode,
            conflict.max_scale,
        );
        if conflict.workload.kind == "Revision" {
            add_badge(graph, &conflict.workload, &message);
            add_graph_warning(graph, &mut warned, &conflict.workload, &message);
        }
        add_badge(graph, &conflict.kservice, &message);
        add_graph_warning(graph, &mut warned, &conflict.kservice, &message);
    }
}

fn add_edge_warning(
    graph: &mut GraphBuilder,
    id: &str,
    warning: Warning,
    access_mode: &knect_proto::storage::AccessMode,
    max_scale: Option<u32>,
) {
    let Some(mut edge) = graph.edge(id).cloned() else {
        return;
    };
    if edge
        .warnings
        .iter()
        .any(|existing| existing.code == warning.code)
    {
        return;
    }
    edge.warnings.push(warning);
    edge.health = Health::Degraded;
    insert_text_metadata(
        &mut edge,
        "conflict_access_mode",
        access_mode_label(*access_mode),
    );
    insert_metadata(&mut edge, "conflict_max_scale", json!(max_scale));
    graph.upsert_edge(edge);
}

fn add_badge(graph: &mut GraphBuilder, id: &ResourceId, message: &str) {
    let Some(mut node) = graph.node(id).cloned() else {
        return;
    };
    if node
        .badges
        .iter()
        .any(|badge| badge.label == CONFLICT_BADGE)
    {
        return;
    }
    node.badges.push(Badge {
        label: CONFLICT_BADGE.to_owned(),
        severity: Severity::High,
        tooltip: Some(message.to_owned()),
    });
    graph.upsert_node(node);
}

fn add_graph_warning(
    graph: &mut GraphBuilder,
    warned: &mut BTreeSet<(String, ResourceId)>,
    resource: &ResourceId,
    message: &str,
) {
    if !warned.insert((CONFLICT_CODE.to_owned(), resource.clone())) {
        return;
    }
    graph.add_warning(Warning {
        code: CONFLICT_CODE.to_owned(),
        message: message.to_owned(),
        severity: Severity::High,
        resource: Some(resource.clone()),
    });
}
