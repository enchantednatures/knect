//! Flagger-owned Istio resources.

use knect_proto::ResourceId;
use knect_proto::graph::EdgeKind;
use kube::ResourceExt;
use serde_json::json;

use crate::graph::GraphBuilder;
use crate::graph::support::{
    add_unique_edge, field, insert_metadata, insert_text_metadata, new_edge, text_field,
};
use crate::store::ResourceStore;

use super::super::support::{ISTIO_NETWORKING_GROUP, ids_by_group_kind, vs_hosts};

pub(super) fn add_managed_edges(
    store: &ResourceStore,
    graph: &mut GraphBuilder,
    canaries: &[ResourceId],
) {
    let mut resources = ids_by_group_kind(store, ISTIO_NETWORKING_GROUP, "VirtualService");
    resources.extend(ids_by_group_kind(
        store,
        ISTIO_NETWORKING_GROUP,
        "DestinationRule",
    ));
    for canary in canaries {
        let canary_uid = store
            .get(canary)
            .and_then(|object| object.metadata.uid.clone());
        for resource in &resources {
            if resource.namespace != canary.namespace || !graph.contains_node(resource) {
                continue;
            }
            let Some(object) = store.get(resource) else {
                continue;
            };
            if !owned_by_canary(&object, canary, canary_uid.as_deref()) {
                continue;
            }
            let mut edge = new_edge(EdgeKind::Manages, canary, resource);
            insert_text_metadata(&mut edge, "managed_by", "flagger");
            insert_text_metadata(&mut edge, "kind", &resource.kind);
            if resource.kind == "VirtualService" {
                insert_metadata(&mut edge, "hosts", json!(vs_hosts(&object)));
            } else if let Some(host) = text_field(&object, &["spec", "host"]) {
                insert_text_metadata(&mut edge, "host", host);
                if let Some(subsets) = field(&object, &["spec", "subsets"]) {
                    insert_metadata(&mut edge, "subsets", subsets.clone());
                }
            }
            add_unique_edge(graph, edge, None);
        }
    }
}

fn owned_by_canary(
    object: &kube::api::DynamicObject,
    canary: &ResourceId,
    canary_uid: Option<&str>,
) -> bool {
    object.owner_references().iter().any(|owner| {
        owner.kind == "Canary"
            && owner.name == canary.name
            && (owner.uid.is_empty() || canary_uid.is_none_or(|uid| uid == owner.uid.as_str()))
    })
}
