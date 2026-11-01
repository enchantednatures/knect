#![allow(clippy::unwrap_used, clippy::expect_used)]

//! ReadWriteOnce autoscaling conflict tests.

mod storage_support;

use knect_proto::graph::{EdgeKind, Health, Severity};
use knect_proto::{NodeCategory, ResourceId};
use serde_json::{Value, json};

use storage_support::*;

#[test]
fn storage_rwo_with_max_scale_five_warns_on_node_edge_and_graph() {
    let topology = build_topology(&load_store());

    let revision = node(&topology, NodeCategory::Revision, "storage-consumer-00001");
    assert!(badge_labels(revision).contains(&"rwo-maxscale-conflict"));
    let service = node(&topology, NodeCategory::KService, "storage-consumer");
    assert!(badge_labels(service).contains(&"rwo-maxscale-conflict"));

    let mount = edge(
        &topology,
        "storage-consumer-00001",
        "storage-data",
        EdgeKind::Mounts,
    );
    assert_eq!(mount.health, Health::Degraded);
    assert_eq!(mount.warnings.len(), 1);
    assert_eq!(mount.warnings[0].code, "rwo_maxscale_conflict");
    assert_eq!(mount.warnings[0].severity, Severity::High);
    assert_eq!(mount.metadata.get("conflict_max_scale"), Some(&json!(5)));
    assert_eq!(
        mount
            .metadata
            .get("conflict_access_mode")
            .and_then(Value::as_str),
        Some("ReadWriteOnce")
    );

    let warnings = conflict_warnings(&topology);
    let resources: Vec<&ResourceId> = warnings
        .iter()
        .filter_map(|warning| warning.resource.as_ref())
        .collect();
    assert!(resources.contains(&&ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Revision",
        NAMESPACE,
        "storage-consumer-00001"
    )));
    assert!(resources.contains(&&ResourceId::namespaced(
        "serving.knative.dev/v1",
        "Service",
        NAMESPACE,
        "storage-consumer"
    )));
    assert!(
        warnings
            .iter()
            .all(|warning| warning.severity == Severity::High)
    );

    for workload in [
        ResourceId::namespaced(
            "apps/v1",
            "Deployment",
            NAMESPACE,
            "storage-consumer-00001-deployment",
        ),
        ResourceId::namespaced("v1", "Pod", NAMESPACE, "storage-consumer-00001-abc"),
    ] {
        let mount = topology
            .edges
            .iter()
            .find(|edge| {
                edge.kind == EdgeKind::Mounts
                    && edge.from == workload
                    && edge.to.name == "storage-data"
            })
            .expect("workload mount edge");
        assert_eq!(mount.warnings.len(), 1);
        assert_eq!(mount.warnings[0].resource.as_ref(), Some(&workload));
    }
}

#[test]
fn storage_rwx_and_single_replica_claims_do_not_warn() {
    let topology = build_topology(&load_store());
    let resources: Vec<&ResourceId> = conflict_warnings(&topology)
        .iter()
        .filter_map(|warning| warning.resource.as_ref())
        .collect();

    for name in ["storage-reader", "storage-single"] {
        let id = ResourceId::namespaced("serving.knative.dev/v1", "Service", NAMESPACE, name);
        assert!(!resources.contains(&&id), "{name} must not warn");
        assert!(
            !badge_labels(node(&topology, NodeCategory::KService, name))
                .contains(&"rwo-maxscale-conflict")
        );
    }
    assert!(
        !badge_labels(node(
            &topology,
            NodeCategory::Revision,
            "storage-reader-00001"
        ))
        .contains(&"rwo-maxscale-conflict")
    );
    assert!(
        !badge_labels(node(
            &topology,
            NodeCategory::Revision,
            "storage-single-00001"
        ))
        .contains(&"rwo-maxscale-conflict")
    );
    assert!(conflict_warnings(&topology).iter().all(|warning| {
        warning
            .resource
            .as_ref()
            .is_none_or(|id| id.name != "storage-reader-00001" && id.name != "storage-single-00001")
    }));
}

#[test]
fn storage_rwo_with_zero_or_unset_max_scale_warns() {
    for value in [Some("0"), None] {
        let mut store = load_store();
        set_max_scale(&mut store, "storage-consumer", value);
        let topology = build_topology(&store);
        let mount = edge(
            &topology,
            "storage-consumer-00001",
            "storage-data",
            EdgeKind::Mounts,
        );
        assert_eq!(mount.warnings.len(), 1, "value {value:?} must warn");
        let expected = value.and_then(|value| value.parse::<u64>().ok());
        assert_eq!(
            mount.metadata.get("conflict_max_scale"),
            Some(&json!(expected))
        );
        assert!(
            badge_labels(node(
                &topology,
                NodeCategory::Revision,
                "storage-consumer-00001"
            ))
            .contains(&"rwo-maxscale-conflict")
        );
    }
}

#[test]
fn storage_read_write_once_pod_is_treated_as_single_writer() {
    let mut store = load_store();
    set_claim_access_modes(&mut store, "storage-data", &["ReadWriteOncePod"]);
    let topology = build_topology(&store);
    let mount = edge(
        &topology,
        "storage-consumer-00001",
        "storage-data",
        EdgeKind::Mounts,
    );
    assert_eq!(mount.warnings.len(), 1);
    assert_eq!(
        mount
            .metadata
            .get("conflict_access_mode")
            .and_then(Value::as_str),
        Some("ReadWriteOncePod")
    );
}
