#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use chrono::{DateTime, Utc};
use knect_proto::error::{ErrorData, NOT_FOUND, RpcError};
use knect_proto::events::{
    ActionResult, DaemonError, GraphPatch, Heartbeat, NodeUpdated, PolicyViolation, SseEvent,
    SseEventKind,
};
use knect_proto::graph::{
    Badge, CategoryCount, Edge, Health, MetricsSummary, NodeCategory, NodeDetails, ResourceNode,
    Severity, Topology, TopologyStats, Warning,
};
use knect_proto::health::{HealthSummary, ScaleState};
use knect_proto::ids::{FieldPath, Gvk, ResourceId};
use knect_proto::links::{Evidence, EvidenceKind, LinkRecord, LinkVia};
use knect_proto::mutation::{
    DiffLine, FieldOwnership, ManagerKind, MutationDiff, MutationDiffMode, MutationOperation,
    MutationRecord, MutationSource,
};
use knect_proto::policy::{
    PolicyKind, PolicyRecord, PolicyReport, PolicyResult, PolicySummary, ValidationResult,
};
use knect_proto::rpc::{
    ConfigGetResult, DaemonStatus, DaemonStatusParams, EmptyParams, EmptyResult,
    EventsSubscribeParams, EventsSubscribeResult, FlaggerAction, FlaggerControlParams,
    FlaggerControlResult, GetDiffParams, GetNodeDetailsParams, GetTopologyParams, JSONRPC_VERSION,
    LogsStopParams, LogsStopResult, LogsSubscribeParams, LogsSubscribeResult, RefreshParams,
    RefreshResult, RpcId, RpcRequest, RpcResponse, SafeActionsConfig, SafeCacheConfig, SafeConfig,
    SafeDaemonConfig, SafeDiscoveryConfig, SafeTapConfig, SafeUiConfig, SafeWatchConfig,
    SetTrafficSplitParams, TapListResult, TapStartParams, TapStopParams, TapStopResult,
    TopologySection, TrafficSplit, ValidateManifestParams, WatcherStatus,
};
use knect_proto::storage::{AccessMode, StorageConflictWarning, StorageRecord, StorageWarning};
use knect_proto::tap::{
    LogLine, LogStream, PodRef, TapEvent, TapFilter, TapHandle, TapState, TapTarget, TapTargetKind,
};

fn roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_value(value).unwrap();
    let decoded: T = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(&decoded, value);
    let reencoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(reencoded, json);
}

fn sample_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-31T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn sample_id() -> ResourceId {
    ResourceId::namespaced("serving.knative.dev/v1", "Service", "default", "hello")
}

fn sample_mutation() -> MutationRecord {
    MutationRecord {
        manager: "knative-serving-controller".to_owned(),
        manager_kind: ManagerKind::Knative,
        operation: MutationOperation::Update,
        subresource: None,
        time: Some(sample_time()),
        paths: vec![FieldOwnership {
            path: FieldPath::new("/spec/timeoutSeconds"),
            yaml_line: Some(15),
            yaml_column: Some(3),
            value_preview: Some("300".to_owned()),
        }],
    }
}

fn sample_policy() -> PolicyRecord {
    PolicyRecord {
        id: "policy-1".to_owned(),
        name: "require-labels".to_owned(),
        namespace: None,
        kind: PolicyKind::Kyverno,
        resource: Some(sample_id()),
        severity: Severity::Medium,
        result: PolicyResult::Warn,
        message: "missing owner label".to_owned(),
        observed_at: Some(sample_time()),
        details: BTreeMap::from([("rule".to_owned(), "check-owner-label".to_owned())]),
    }
}

fn sample_link() -> LinkRecord {
    LinkRecord {
        source: sample_id(),
        target: ResourceId::namespaced("apps/v1", "Deployment", "default", "redis-master"),
        via: LinkVia::Env,
        confidence: 0.86,
        evidence: vec![Evidence {
            kind: EvidenceKind::EnvVar,
            description: "REDIS_URL points at redis-master".to_owned(),
            path: Some(FieldPath::new("/spec/containers/0/env/0/value")),
            value_redacted: Some("[REDACTED]".to_owned()),
        }],
        health: Health::Healthy,
    }
}

fn sample_storage() -> StorageRecord {
    let pvc = ResourceId::namespaced("v1", "PersistentVolumeClaim", "default", "redis-data");
    StorageRecord {
        pvc: pvc.clone(),
        volume: Some("redis-data".to_owned()),
        storage_class: Some("ceph-rbd".to_owned()),
        access_modes: vec![AccessMode::ReadWriteOnce],
        capacity: Some("20Gi".to_owned()),
        phase: Some("Bound".to_owned()),
        mounted_by: vec![sample_id()],
        provisioned_by: Some(ResourceId::cluster_scoped(
            "storage.k8s.io/v1",
            "StorageClass",
            "ceph-rbd",
        )),
        backend: Some(ResourceId::namespaced(
            "ceph.rook.io/v1",
            "CephBlockPool",
            "rook-ceph",
            "replicated",
        )),
        warnings: vec![StorageWarning {
            code: "rwo_multi_mount".to_owned(),
            message: "RWO claim mounted by multiple replicas".to_owned(),
            severity: Severity::High,
            conflict: Some(StorageConflictWarning {
                pvc,
                access_mode: AccessMode::ReadWriteOnce,
                max_scale: Some(2),
                reason: "RWO volumes cannot attach to multiple nodes".to_owned(),
                affected: vec![sample_id()],
            }),
        }],
    }
}

fn sample_log_line() -> LogLine {
    LogLine {
        namespace: "default".to_owned(),
        pod: "hello-00001-deployment-abc".to_owned(),
        container: "user-container".to_owned(),
        timestamp: sample_time(),
        message: "request handled in 12ms".to_owned(),
        stream: LogStream::Stdout,
    }
}

fn sample_tap_handle() -> TapHandle {
    TapHandle {
        id: "tap-1".to_owned(),
        target: TapTarget {
            kind: TapTargetKind::Broker,
            namespace: "default".to_owned(),
            name: "orders".to_owned(),
        },
        state: TapState::Active,
        created: sample_time(),
        expires: sample_time(),
        max_events: 1000,
        callback_url: "http://knectd.default.svc:7421/tap/tap-1".to_owned(),
        filter: Some(TapFilter {
            attributes: BTreeMap::from([("type".to_owned(), "order.created".to_owned())]),
        }),
    }
}

fn sample_tap_event() -> TapEvent {
    TapEvent {
        id: "event-1".to_owned(),
        ce_id: "ce-1".to_owned(),
        ce_source: "/apis/eventing.knative.dev/v1/namespaces/default/brokers/orders".to_owned(),
        ce_type: "order.created".to_owned(),
        ce_subject: Some("orders/123".to_owned()),
        ce_time: Some(sample_time()),
        headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        payload: serde_json::json!({"order_id": "123"}),
        received_at: sample_time(),
    }
}

fn sample_node() -> ResourceNode {
    ResourceNode {
        id: sample_id(),
        category: NodeCategory::KService,
        name: "hello".to_owned(),
        namespace: Some("default".to_owned()),
        health: Health::Healthy,
        summary: "Knative Service serving hello".to_owned(),
        labels: BTreeMap::from([("app".to_owned(), "hello".to_owned())]),
        annotations: BTreeMap::new(),
        badges: vec![Badge {
            label: "scale-to-zero".to_owned(),
            severity: Severity::Info,
            tooltip: None,
        }],
        uid: Some("uid-1".to_owned()),
        first_seen: sample_time(),
        last_seen: sample_time(),
    }
}

fn sample_edge() -> Edge {
    Edge {
        id: "edge-1".to_owned(),
        from: sample_id(),
        to: ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Revision",
            "default",
            "hello-00001",
        ),
        kind: knect_proto::graph::EdgeKind::Owns,
        label: Some("revision".to_owned()),
        health: Health::Healthy,
        warnings: vec![Warning {
            code: "example".to_owned(),
            message: "example warning".to_owned(),
            severity: Severity::Low,
            resource: Some(sample_id()),
        }],
        metadata: BTreeMap::from([("route".to_owned(), serde_json::json!("hello"))]),
    }
}

fn sample_topology() -> Topology {
    Topology {
        schema_version: knect_proto::SCHEMA_VERSION,
        context: "kind-knect-e2e".to_owned(),
        generation: 42,
        nodes: vec![sample_node()],
        edges: vec![sample_edge()],
        warnings: vec![Warning {
            code: "partial".to_owned(),
            message: "one GVK unavailable".to_owned(),
            severity: Severity::Medium,
            resource: None,
        }],
        stats: TopologyStats {
            node_count: 1,
            edge_count: 1,
            warning_count: 1,
            categories: vec![CategoryCount {
                category: NodeCategory::KService,
                count: 1,
            }],
            truncated: false,
        },
    }
}

fn sample_details() -> NodeDetails {
    NodeDetails {
        id: sample_id(),
        category: NodeCategory::KService,
        name: "hello".to_owned(),
        namespace: Some("default".to_owned()),
        health: Health::Healthy,
        summary: "Knative Service serving hello".to_owned(),
        yaml: Some("apiVersion: serving.knative.dev/v1\nkind: Service\n".to_owned()),
        yaml_redacted: true,
        mutations: vec![sample_mutation()],
        policies: vec![sample_policy()],
        links: vec![sample_link()],
        storage: vec![sample_storage()],
        warnings: vec![Warning {
            code: "example".to_owned(),
            message: "example warning".to_owned(),
            severity: Severity::Low,
            resource: Some(sample_id()),
        }],
        metrics: Some(MetricsSummary {
            cpu_millicores: Some(12.5),
            memory_bytes: Some(67_108_864),
            replicas: Some(1),
            ready_replicas: Some(1),
            requests_per_second: Some(23.4),
            error_rate: Some(0.001),
            p95_latency_ms: Some(41.2),
            window_secs: Some(300),
            source: Some("kubernetes-metrics".to_owned()),
        }),
        logs: vec![sample_log_line()],
    }
}

fn sample_action_result() -> ActionResult {
    ActionResult {
        action: "actions.set_traffic_split".to_owned(),
        success: true,
        message: "traffic split updated".to_owned(),
        dry_run: false,
        resource: Some(sample_id()),
        generation: Some(43),
        warnings: vec![Warning {
            code: "example".to_owned(),
            message: "example warning".to_owned(),
            severity: Severity::Low,
            resource: Some(sample_id()),
        }],
        ts: sample_time(),
    }
}

fn sample_safe_config() -> SafeConfig {
    SafeConfig {
        daemon: SafeDaemonConfig {
            socket_path: "/tmp/knectd.sock".to_owned(),
            http_addr: "127.0.0.1:7420".to_owned(),
            cache_dir: "~/.cache/knect".to_owned(),
            context: "kind-knect-e2e".to_owned(),
            log: "info".to_owned(),
            log_format: "human".to_owned(),
            allow_actions: false,
            allow_remote: false,
        },
        discovery: SafeDiscoveryConfig {
            rediscover_secs: 60,
            groups: vec!["serving.knative.dev".to_owned()],
            namespaces: Vec::new(),
        },
        watch: SafeWatchConfig {
            debounce_ms: 150,
            max_objects: 20_000,
            strip_annotations: vec!["kubectl.kubernetes.io/last-applied-configuration".to_owned()],
        },
        cache: SafeCacheConfig {
            enabled: true,
            flush_ms: 500,
            max_snapshot_bytes: 33_554_432,
        },
        tap: SafeTapConfig {
            enabled: true,
            callback_bind: "0.0.0.0:7421".to_owned(),
            callback_host: "knectd.default.svc".to_owned(),
            max_taps: 5,
            max_events: 1000,
            ttl_secs: 300,
        },
        ui: SafeUiConfig {
            fps: 60,
            theme: "knect-dark".to_owned(),
            ascii: false,
        },
        actions: SafeActionsConfig { enabled: false },
    }
}

#[test]
fn identifiers_roundtrip() {
    roundtrip(&Gvk::from_api_version("serving.knative.dev/v1", "Service"));
    roundtrip(&Gvk::new("", "v1", "Service"));
    roundtrip(&sample_id());
    roundtrip(&ResourceId::cluster_scoped(
        "storage.k8s.io/v1",
        "StorageClass",
        "standard",
    ));
    roundtrip(&FieldPath::new(
        "/spec/template/metadata/annotations/sidecar.istio.io~1inject",
    ));
    assert_eq!(FieldPath::new("").segments().count(), 0);
    assert_eq!(FieldPath::new("/a/b").segments().count(), 2);
}

#[test]
fn graph_enums_roundtrip() {
    for category in [
        NodeCategory::KService,
        NodeCategory::Route,
        NodeCategory::Configuration,
        NodeCategory::Revision,
        NodeCategory::PodAutoscaler,
        NodeCategory::Broker,
        NodeCategory::Trigger,
        NodeCategory::Source,
        NodeCategory::SinkBinding,
        NodeCategory::Subscription,
        NodeCategory::Channel,
        NodeCategory::Deployment,
        NodeCategory::ReplicaSet,
        NodeCategory::Pod,
        NodeCategory::Service,
        NodeCategory::Endpoints,
        NodeCategory::BackingService,
        NodeCategory::Topic,
        NodeCategory::StorageClass,
        NodeCategory::PersistentVolume,
        NodeCategory::PersistentVolumeClaim,
        NodeCategory::CephPool,
        NodeCategory::Policy,
        NodeCategory::Canary,
        NodeCategory::VirtualService,
        NodeCategory::DestinationRule,
        NodeCategory::PeerAuthentication,
        NodeCategory::AuthorizationPolicy,
        NodeCategory::ConfigMap,
        NodeCategory::Secret,
        NodeCategory::Namespace,
        NodeCategory::Unknown,
    ] {
        roundtrip(&category);
    }
    for kind in [
        knect_proto::graph::EdgeKind::RoutesTo,
        knect_proto::graph::EdgeKind::Owns,
        knect_proto::graph::EdgeKind::Scales,
        knect_proto::graph::EdgeKind::Runs,
        knect_proto::graph::EdgeKind::Subscribes,
        knect_proto::graph::EdgeKind::DeliversTo,
        knect_proto::graph::EdgeKind::DeadLetterSink,
        knect_proto::graph::EdgeKind::UsesSecret,
        knect_proto::graph::EdgeKind::UsesConfigMap,
        knect_proto::graph::EdgeKind::ConnectsTo,
        knect_proto::graph::EdgeKind::BackedBy,
        knect_proto::graph::EdgeKind::Mounts,
        knect_proto::graph::EdgeKind::ProvisionedBy,
        knect_proto::graph::EdgeKind::ProtectedBy,
        knect_proto::graph::EdgeKind::MutatedBy,
        knect_proto::graph::EdgeKind::Manages,
        knect_proto::graph::EdgeKind::References,
        knect_proto::graph::EdgeKind::Targets,
        knect_proto::graph::EdgeKind::PublishedBy,
        knect_proto::graph::EdgeKind::ConsumesFrom,
        knect_proto::graph::EdgeKind::Selects,
        knect_proto::graph::EdgeKind::Unknown,
    ] {
        roundtrip(&kind);
    }
    for health in [
        Health::Healthy,
        Health::Degraded,
        Health::Unknown,
        Health::Progressing,
        Health::Blocked,
    ] {
        roundtrip(&health);
    }
    for severity in [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
        Severity::Unknown,
    ] {
        roundtrip(&severity);
    }
}

#[test]
fn topology_roundtrip() {
    roundtrip(&sample_node());
    roundtrip(&sample_edge());
    roundtrip(&sample_topology());
}

#[test]
fn node_details_roundtrip() {
    roundtrip(&sample_details());
    roundtrip(&HealthSummary {
        health: Health::Degraded,
        scale_state: ScaleState::ScalingUp,
        ready_replicas: Some(1),
        desired_replicas: Some(2),
        reasons: vec!["scale up requested".to_owned()],
        last_transition: Some(sample_time()),
    });
}

#[test]
fn mutation_roundtrip() {
    roundtrip(&sample_mutation());
    roundtrip(&MutationDiff {
        resource: sample_id(),
        user_intent: Some("kind: Service\n".to_owned()),
        knative_defaults: Some("kind: Service\nmetadata: {}\n".to_owned()),
        policy_mutators: Some("kind: Service\nspec: {}\n".to_owned()),
        lines: vec![DiffLine {
            line_number: 1,
            side: MutationSource::UserIntent,
            owner: Some("kubectl".to_owned()),
            text: "kind: Service".to_owned(),
            changed: false,
            path: Some(FieldPath::new("/kind")),
        }],
    });
    roundtrip(&MutationDiffMode::ThreeWay);
    roundtrip(&MutationDiffMode::Managers);
}

#[test]
fn policy_roundtrip() {
    roundtrip(&sample_policy());
    roundtrip(&PolicyReport {
        resource: Some(sample_id()),
        results: vec![sample_policy()],
        summary: PolicySummary {
            pass: 3,
            fail: 0,
            warn: 1,
            error: 0,
            skip: 0,
        },
        generated_at: Some(sample_time()),
    });
    roundtrip(&ValidationResult {
        valid: false,
        mutated_manifest: Some("kind: Service\n".to_owned()),
        violations: vec![sample_policy()],
        warnings: vec![Warning {
            code: "example".to_owned(),
            message: "example warning".to_owned(),
            severity: Severity::Low,
            resource: None,
        }],
    });
}

#[test]
fn link_roundtrip() {
    roundtrip(&sample_link());
    roundtrip(&EvidenceKind::SecretKey);
    roundtrip(&LinkVia::Dns);
}

#[test]
fn storage_roundtrip() {
    roundtrip(&sample_storage());
    roundtrip(&AccessMode::ReadWriteMany);
    roundtrip(&StorageConflictWarning {
        pvc: ResourceId::namespaced("v1", "PersistentVolumeClaim", "default", "data"),
        access_mode: AccessMode::ReadWriteOnce,
        max_scale: Some(3),
        reason: "conflict".to_owned(),
        affected: vec![sample_id()],
    });
}

#[test]
fn tap_roundtrip() {
    roundtrip(&sample_tap_handle());
    roundtrip(&sample_tap_event());
    roundtrip(&sample_log_line());
    roundtrip(&PodRef {
        namespace: "default".to_owned(),
        name: "hello-00001-deployment-abc".to_owned(),
        container: Some("user-container".to_owned()),
    });
}

#[test]
fn rpc_envelopes_roundtrip() {
    roundtrip(&RpcId::Number(1));
    roundtrip(&RpcId::String("req-1".to_owned()));
    roundtrip(&RpcId::Null);
    roundtrip(&RpcRequest::new(
        1,
        "graph.get_topology",
        Some(serde_json::json!({"include": ["nodes"]})),
    ));
    roundtrip(&RpcRequest::notification("events.subscribe", None));
    roundtrip(&RpcResponse::success(
        RpcId::String("req-1".to_owned()),
        serde_json::json!({"generation": 42}),
    ));
    roundtrip(&RpcResponse::failure(
        RpcId::Number(7),
        RpcError::with_data(
            NOT_FOUND,
            "resource not found",
            ErrorData {
                retryable: false,
                details: Some("missing".to_owned()),
                resource: Some(sample_id()),
            },
        ),
    ));
}

#[test]
fn rpc_params_and_results_roundtrip() {
    roundtrip(&EmptyParams {});
    roundtrip(&EmptyResult {});
    roundtrip(&WatcherStatus {
        gvk: Gvk::from_api_version("serving.knative.dev/v1", "Service"),
        resource_count: 3,
        healthy: true,
        last_event: Some(sample_time()),
    });
    roundtrip(&DaemonStatus {
        version: "0.1.0".to_owned(),
        schema_version: knect_proto::SCHEMA_VERSION,
        context: "kind-knect-e2e".to_owned(),
        generation: 42,
        watchers: vec![WatcherStatus {
            gvk: Gvk::from_api_version("serving.knative.dev/v1", "Service"),
            resource_count: 3,
            healthy: true,
            last_event: Some(sample_time()),
        }],
        uptime_secs: 120,
        actions_enabled: false,
        tap_count: 0,
        started_at: Some(sample_time()),
        cache_enabled: true,
        last_resync: Some(sample_time()),
    });
    roundtrip(&DaemonStatusParams {});
    roundtrip(&GetTopologyParams {
        context: Some("kind-knect-e2e".to_owned()),
        min_generation: Some(42),
        include: Some(vec![TopologySection::Nodes, TopologySection::Edges]),
    });
    roundtrip(&GetNodeDetailsParams {
        id: sample_id(),
        include_yaml: Some(true),
        include_logs: Some(false),
    });
    roundtrip(&RefreshParams {
        rediscover: Some(true),
    });
    roundtrip(&RefreshResult { generation: 43 });
    roundtrip(&GetDiffParams {
        id: sample_id(),
        mode: Some(MutationDiffMode::Managers),
    });
    roundtrip(&TrafficSplit {
        revision: "hello-00002".to_owned(),
        percent: 25,
        tag: Some("canary".to_owned()),
    });
    roundtrip(&SetTrafficSplitParams {
        namespace: "default".to_owned(),
        service: "hello".to_owned(),
        splits: vec![TrafficSplit {
            revision: "hello-00002".to_owned(),
            percent: 100,
            tag: None,
        }],
        dry_run: Some(true),
    });
    roundtrip(&FlaggerControlParams {
        namespace: "default".to_owned(),
        canary: "hello".to_owned(),
        action: FlaggerAction::Promote,
        dry_run: Some(false),
    });
    roundtrip(&FlaggerControlResult {
        ..sample_action_result()
    });
    roundtrip(&ValidateManifestParams {
        manifest: "kind: Service\n".to_owned(),
        namespace: Some("default".to_owned()),
        default_kind: Some("Service".to_owned()),
    });
    roundtrip(&TapStartParams {
        target: TapTarget {
            kind: TapTargetKind::Trigger,
            namespace: "default".to_owned(),
            name: "orders-to-hello".to_owned(),
        },
        filter: Some(TapFilter {
            attributes: BTreeMap::from([("type".to_owned(), "order.created".to_owned())]),
        }),
        max_events: Some(100),
        ttl_secs: Some(60),
    });
    roundtrip(&TapStopParams {
        tap_id: "tap-1".to_owned(),
    });
    roundtrip(&TapStopResult { removed: true });
    let taps: TapListResult = vec![sample_tap_handle()];
    roundtrip(&taps);
    roundtrip(&LogsSubscribeParams {
        pods: vec![PodRef {
            namespace: "default".to_owned(),
            name: "hello-00001-deployment-abc".to_owned(),
            container: None,
        }],
        tail_lines: Some(100),
        since: Some(sample_time()),
        filter: Some("error".to_owned()),
    });
    roundtrip(&LogsSubscribeResult {
        subscription_id: "logs-1".to_owned(),
    });
    roundtrip(&LogsStopParams {
        subscription_id: "logs-1".to_owned(),
    });
    roundtrip(&LogsStopResult { stopped: true });
    roundtrip(&EventsSubscribeParams {
        topics: Some(vec!["graph.patch".to_owned()]),
        last_event_id: Some(41),
    });
    roundtrip(&EventsSubscribeResult {
        subscription_id: "events-1".to_owned(),
    });
    let config: ConfigGetResult = sample_safe_config();
    roundtrip(&config);
    assert_eq!(JSONRPC_VERSION, "2.0");
}

#[test]
fn events_roundtrip() {
    let topology = sample_topology();
    let graph_patch = GraphPatch {
        generation: 42,
        added_nodes: vec![sample_node()],
        updated_nodes: Vec::new(),
        removed_nodes: Vec::new(),
        added_edges: vec![sample_edge()],
        removed_edges: Vec::new(),
        seq: 42,
        ts: sample_time(),
    };
    let node_updated = NodeUpdated {
        node: sample_node(),
        changed_fields: vec!["summary".to_owned()],
        generation: 42,
        ts: sample_time(),
    };
    let policy_violation = PolicyViolation {
        policy: sample_policy(),
        generation: 42,
        ts: sample_time(),
    };
    let action_result = sample_action_result();
    let heartbeat = Heartbeat {
        uptime_secs: 120,
        generation: 42,
        node_count: 4,
        edge_count: 3,
        ts: sample_time(),
    };
    let daemon_error = DaemonError {
        code: NOT_FOUND,
        message: "resource disappeared".to_owned(),
        retryable: true,
        ts: sample_time(),
    };

    let events = [
        (
            "snapshot",
            SseEventKind::Snapshot {
                data: topology.clone(),
            },
        ),
        (
            "graph.patch",
            SseEventKind::GraphPatch {
                data: graph_patch.clone(),
            },
        ),
        (
            "node.updated",
            SseEventKind::NodeUpdated {
                data: node_updated.clone(),
            },
        ),
        (
            "policy.violation",
            SseEventKind::PolicyViolation {
                data: policy_violation.clone(),
            },
        ),
        (
            "tap.event",
            SseEventKind::TapEvent {
                data: sample_tap_event(),
            },
        ),
        (
            "log.line",
            SseEventKind::LogLine {
                data: sample_log_line(),
            },
        ),
        (
            "action.result",
            SseEventKind::ActionResult {
                data: action_result.clone(),
            },
        ),
        (
            "daemon.heartbeat",
            SseEventKind::Heartbeat {
                data: heartbeat.clone(),
            },
        ),
        (
            "daemon.error",
            SseEventKind::DaemonError {
                data: daemon_error.clone(),
            },
        ),
    ];

    for (expected_name, kind) in events {
        let event = SseEvent::new(42, sample_time(), kind.clone());
        assert_eq!(event.event_type(), expected_name);
        roundtrip(&event);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], expected_name);
        assert!(json.get("data").is_some());
        roundtrip(&kind);
    }

    roundtrip(&graph_patch);
    roundtrip(&node_updated);
    roundtrip(&policy_violation);
    roundtrip(&action_result);
    roundtrip(&heartbeat);
    roundtrip(&daemon_error);
}

#[test]
fn rpc_envelopes_accept_omitted_optional_fields() {
    let request: RpcRequest =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"tap.list"}"#).unwrap();
    assert_eq!(request.id, RpcId::Number(1));
    assert!(request.params.is_none());

    let response: RpcResponse = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1}"#).unwrap();
    assert_eq!(response.id, RpcId::Number(1));
    assert!(response.result.is_none());
    assert!(response.error.is_none());
}
