# knect protocol v1

Status: **frozen contract**, schema version **1**.

This document is the authoritative wire contract between `knectd` and every
client: the `knect` TUI/CLI, the WebUI, the Neovim plugin, and tests. The Rust
definitions live in `crates/knect-proto`; the language-neutral golden payloads
live in `testdata/contracts/`. If this document and the code disagree, the code
and its tests are the defect — fix both in the same change.

The contract is additive. New optional fields may be added inside a schema
version. Field names, enum tags, method names, event names, and error codes are
never renamed or removed without bumping `SCHEMA_VERSION`.

---

## 1. Transport

### 1.1 Unix domain socket

- Path: `/tmp/knectd.sock` by default; configured with `daemon.socket_path`.
- Permissions: mode `0600`. The daemon checks the peer UID on Linux and rejects
  peers that do not match its own UID.
- Two framings are accepted on the same socket. The daemon peeks the first byte:
  - `{` or whitespace → **newline-delimited JSON-RPC**: exactly one JSON-RPC
    request object per line, terminated by `\n`. Responses and subscriptions are
    also one JSON object per line.
  - Anything else → **HTTP/1.1** handled exactly like the TCP listener.
- `events.subscribe` is available over UDS JSONL. Events arriving on the
  subscription are written as `SseEvent` JSON objects, one per line, with
  `seq`/`ts`/`kind`/`data`.

### 1.2 HTTP

- Default bind: `127.0.0.1:7420` (`daemon.http_addr`).
- `POST /rpc` — one JSON-RPC request object, `Content-Type: application/json`.
  The response is one JSON-RPC response object. Batch arrays are not accepted.
- `GET /events` — SSE stream (`text/event-stream`).
- `GET /healthz` — `200 OK` with `{"status":"ok"}` once the daemon is serving.
- Non-loopback binds require `daemon.allow_remote = true`. In-cluster Helm sets
  `KNECT_DAEMON_HTTP_ADDR=0.0.0.0:7420`; the Service still fronts it.
- Credentials, bearer tokens, and kubeconfig contents are never served.

### 1.3 JSON-RPC envelope

Single request, single response. `jsonrpc` is always `"2.0"`.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "graph.get_topology",
  "params": {
    "context": "kind-knect-e2e",
    "min_generation": 42,
    "include": ["nodes", "edges", "warnings", "stats"]
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": { "schema_version": 1, "context": "kind-knect-e2e", "generation": 42, "nodes": [], "edges": [], "warnings": [], "stats": { "node_count": 0, "edge_count": 0, "warning_count": 0, "categories": [], "truncated": false } },
  "error": null
}
```

- `id` is a number, a string, or `null`. A `null` ID marks a notification; the
  daemon still processes it but does not reply.
- Exactly one of `result` and `error` is non-null. `result` may legitimately be
  `null` for a successful method that returns nothing; clients distinguish
  success by `error == null`.
- Unknown methods return `-32601`. Malformed params return `-32602`.

### 1.4 SSE

`GET /events` returns `text/event-stream`. Every frame is:

```text
id: <seq>
event: <event-name>
data: <one-line SseEvent JSON>

```

- `id` equals `SseEvent.seq`.
- `data` is a single-line JSON object. Clients must not assume pretty printing.
- The daemon keeps a 1,000-event replay ring. `Last-Event-ID` replays the events
  after that sequence number.
- Heartbeats are emitted every 15 seconds as `daemon.heartbeat`.
- A lagging client that falls out of the replay ring receives a fresh
  `snapshot` instead of an unbounded backlog.

---

## 2. Methods

All params and results are JSON objects unless noted otherwise. Names are a
frozen part of the contract and are exported as constants from
`knect_proto::rpc::methods`.

| Method | Params | Result | Notes |
|---|---|---|---|
| `daemon.status` | `{}` | `DaemonStatus` | Version, schema version, context, generation, watcher health, uptime, action/tap state |
| `graph.get_topology` | `GetTopologyParams` | `Topology` | `min_generation` waits briefly for catch-up; `include` filters sections |
| `graph.get_node_details` | `GetNodeDetailsParams` | `NodeDetails` | Redacted YAML, links, policies, storage, mutations, optional logs/metrics |
| `graph.refresh` | `RefreshParams` | `RefreshResult` | `{ "generation": <u64> }`; `rediscover` forces API discovery |
| `mutations.get_diff` | `GetDiffParams` | `MutationDiff` | `three_way` (default) or `managers` |
| `actions.set_traffic_split` | `SetTrafficSplitParams` | `ActionResult` | Requires actions enabled; percentages must sum to 100; revision ownership validated |
| `actions.flagger_control` | `FlaggerControlParams` | `ActionResult` | `promote` sets `skipAnalysis=true`; `abort` sets `suspend=true, skipAnalysis=false` |
| `policy.validate_manifest` | `ValidateManifestParams` | `ValidationResult` | Server-side `dryRun=All`; returns mutated YAML and policy findings |
| `tap.start` | `TapStartParams` | `TapHandle` | Creates an ephemeral Trigger and token-guarded callback |
| `tap.stop` | `TapStopParams` | `TapStopResult` | Idempotent |
| `tap.list` | `{}` | `TapHandle[]` | Active and recently expired taps |
| `logs.subscribe` | `LogsSubscribeParams` | `LogsSubscribeResult` | Subsequent lines arrive as `log.line` events |
| `logs.stop` | `LogsStopParams` | `LogsStopResult` | Stops one subscription |
| `events.subscribe` | `EventsSubscribeParams` | `EventsSubscribeResult` | **UDS JSONL only**; HTTP clients use `GET /events` |
| `config.get` | `{}` | `SafeConfig` | Safe configuration only; never tokens or secret values |

### 2.1 Params and results

- `GetTopologyParams`: `context?`, `min_generation?`, `include?` where
  `include` is any of `nodes`, `edges`, `warnings`, `stats`.
- `GetNodeDetailsParams`: `id` (`ResourceId`), `include_yaml?`,
  `include_logs?`.
- `RefreshParams`: `rediscover?`. `RefreshResult`: `generation`.
- `GetDiffParams`: `id`, `mode?` (`three_way` | `managers`).
- `SetTrafficSplitParams`: `namespace`, `service`, `splits`
  (`[{ "revision", "percent", "tag"? }]`), `dry_run?`.
- `FlaggerControlParams`: `namespace`, `canary`, `action` (`promote` |
  `abort`), `dry_run?`.
- `ValidateManifestParams`: `manifest`, `namespace?`, `default_kind?`.
- `TapStartParams`: `target` (`{ "kind": "broker"|"trigger", "namespace",
  "name" }`), `filter?` (`{ "attributes": { ... } }`), `max_events?`,
  `ttl_secs?`. Result is a `TapHandle`.
- `TapStopParams`: `tap_id`; `TapStopResult`: `removed`.
- `LogsSubscribeParams`: `pods` (`[{ "namespace", "name", "container"? }]`),
  `tail_lines?`, `since?`, `filter?`; result contains `subscription_id`.
- `LogsStopParams`: `subscription_id`; `LogsStopResult`: `stopped`.
- `EventsSubscribeParams`: `topics?`, `last_event_id?`; result contains
  `subscription_id`.

### 2.2 Full example: `daemon.status`

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "daemon.status",
  "params": {}
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "version": "0.1.0",
    "schema_version": 1,
    "context": "kind-knect-e2e",
    "generation": 42,
    "watchers": [
      {
        "gvk": {
          "group": "serving.knative.dev",
          "version": "v1",
          "kind": "Service"
        },
        "resource_count": 3,
        "healthy": true,
        "last_event": "2026-10-31T12:05:00Z"
      }
    ],
    "uptime_secs": 120,
    "actions_enabled": false,
    "tap_count": 0,
    "started_at": "2026-10-31T12:03:00Z",
    "cache_enabled": true,
    "last_resync": "2026-10-31T12:04:00Z"
  },
  "error": null
}
```

---

## 3. Events

SSE event names and the `SseEventKind` tags are identical. Event constants are
exported from `knect_proto::events::event_names`.

| Event | Payload | Meaning |
|---|---|---|
| `snapshot` | `Topology` | Full replacement; sent on connect, replay overflow, or lag |
| `graph.patch` | `GraphPatch` | Incremental added/updated/removed nodes and edges |
| `node.updated` | `NodeUpdated` | One node changed with changed-field names |
| `policy.violation` | `PolicyViolation` | A policy finding appeared or changed |
| `tap.event` | `TapEvent` | A CloudEvent captured by a tap |
| `log.line` | `LogLine` | One subscribed log line |
| `action.result` | `ActionResult` | A guarded write action completed |
| `daemon.heartbeat` | `Heartbeat` | Liveness, generation, and graph counts every 15 s |
| `daemon.error` | `DaemonError` | Daemon-level failure; `retryable` indicates recovery |

`SseEvent` is:

```json
{
  "seq": 42,
  "ts": "2026-10-31T12:05:00Z",
  "kind": "graph.patch",
  "data": { }
}
```

The SSE frame for that event is in `testdata/contracts/sse_patch.sample.txt`.

---

## 4. Error model

An error response carries `RpcError`:

```json
{
  "code": -32001,
  "message": "resource not found",
  "data": {
    "retryable": false,
    "details": "Revision orders-v2 is not present in the current graph",
    "resource": {
      "api_version": "serving.knative.dev/v1",
      "kind": "Revision",
      "namespace": "orders",
      "name": "orders-v2"
    }
  }
}
```

| Code | Name | Meaning |
|---|---|---|
| `-32700` | `parse_error` | Invalid JSON |
| `-32600` | `invalid_request` | Valid JSON, invalid JSON-RPC envelope |
| `-32601` | `method_not_found` | Unknown method |
| `-32602` | `invalid_params` | Params failed validation; also used for schema version mismatch |
| `-32603` | `internal_error` | Unexpected daemon failure |
| `-32001` | `not_found` | Resource or object not found |
| `-32002` | `conflict` | Concurrent modification or ownership conflict |
| `-32003` | `kube_unavailable` | Kubernetes API unavailable; usually retryable |
| `-32004` | `policy_blocked` | Policy engine blocked the action |
| `-32005` | `unsupported_gvk` | GVK not discovered or not supported |
| `-32006` | `actions_disabled` | Write actions are disabled |
| `-32007` | `tap_limit` | Maximum concurrent taps reached |
| `-32008` | `tap_forbidden` | Tap target or callback is forbidden |
| `-32009` | `cache_corrupt` | Snapshot cache is corrupt; daemon rebuilds |
| `-32010` | `redaction_violation` | Redaction cannot be guaranteed; request refused |
| `-32011` | `timeout` | Operation exceeded its deadline |
| `-32012` | `cancelled` | Operation was cancelled (shutdown or client disconnect) |

`retryable` is advisory. Clients must still back off. A schema version mismatch
is reported as `-32602` with details naming the client and daemon versions.

---

## 5. Core payloads

### 5.1 Identifiers

- `Gvk`: `{ "group", "version", "kind" }`. The core group is `""`.
- `ResourceId`: `{ "api_version", "kind", "namespace", "name" }`.
  `namespace` is `null` for cluster-scoped resources. Node IDs and edge
  endpoints use `ResourceId`; the Kubernetes UID is separate metadata
  (`ResourceNode.uid`).
- `FieldPath`: an RFC 6901 JSON Pointer string, for example
  `/spec/template/spec/containers/0/image`.

### 5.2 Graph

- `ResourceNode`: `id`, `category`, `name`, `namespace`, `health`, `summary`,
  `labels`, `annotations`, `badges`, `uid`, `first_seen`, `last_seen`.
- `Edge`: `id`, `from`, `to`, `kind`, `label`, `health`, `warnings`, `metadata`.
- `Topology`: `schema_version`, `context`, `generation`, `nodes`, `edges`,
  `warnings`, `stats`.
- `TopologyStats`: `node_count`, `edge_count`, `warning_count`, `categories`,
  `truncated`. `categories` is a sorted `[{ "category", "count" }]` list.
- `Health`: `healthy`, `degraded`, `unknown`, `progressing`, `blocked`.
- `Severity`: `critical`, `high`, `medium`, `low`, `info`, `unknown`.
- `GraphPatch`: `generation`, `added_nodes`, `updated_nodes`, `removed_nodes`,
  `added_edges`, `removed_edges`, `seq`, `ts`.
- `NodeUpdated`: `node`, `changed_fields`, `generation`, `ts`.

`NodeCategory` is one of: `k_service`, `route`, `configuration`, `revision`,
`pod_autoscaler`, `broker`, `trigger`, `event_type`, `source`, `sink_binding`,
`subscription`, `channel`, `sink`, `deployment`, `replica_set`, `pod`,
`service`, `endpoints`, `backing_service`, `topic`, `storage_class`,
`persistent_volume`,
`persistent_volume_claim`, `ceph_pool`, `policy`, `canary`, `virtual_service`,
`destination_rule`, `peer_authentication`, `authorization_policy`, `config_map`,
`secret`, `namespace`, `unknown`.

`EdgeKind` is one of: `routes_to`, `owns`, `scales`, `runs`, `subscribes`,
`delivers_to`, `dead_letter_sink`, `uses_secret`, `uses_config_map`,
`connects_to`, `backed_by`, `mounts`, `provisioned_by`, `protected_by`,
`mutated_by`, `manages`, `references`, `targets`, `published_by`,
`consumes_from`, `selects`, `unknown`.

### 5.3 Node details

`NodeDetails` carries `id`, `category`, `name`, `namespace`, `health`,
`summary`, `yaml`, `yaml_redacted`, `mutations`, `policies`, `links`,
`storage`, `warnings`, `metrics`, `logs`. `yaml` is `null` unless requested and
has already passed through the daemon redactor; `yaml_redacted` must be `true`
for any non-null YAML.

### 5.4 Mutations

- `MutationRecord`: `manager`, `manager_kind`, `operation`, `subresource`,
  `time`, `paths`.
- `FieldOwnership`: `path`, `yaml_line`, `yaml_column`, `value_preview`.
  `value_preview` is redacted.
- `MutationDiff`: `resource`, `user_intent`, `knative_defaults`,
  `policy_mutators`, `lines`.
- `DiffLine`: `line_number`, `side`, `owner`, `text`, `changed`, `path`.
  `side` is `user_intent`, `knative_defaults`, or `policy_mutators`.

### 5.5 Links and storage

- `LinkRecord`: `source`, `target`, `via`, `confidence` (`0.0..=1.0`),
  `evidence`, `health`.
- `LinkVia`: `explicit_binding`, `env`, `secret`, `config_map`, `dns`,
  `operator_crd`, `storage`.
- `Evidence`: `kind`, `description`, `path`, `value_redacted`. Raw Secret and
  ConfigMap values never appear.
- `StorageRecord`: `pvc`, `volume`, `storage_class`, `access_modes`, `capacity`,
  `phase`, `mounted_by`, `provisioned_by`, `backend`, `warnings`.
- `StorageConflictWarning`: `pvc`, `access_mode`, `max_scale`, `reason`,
  `affected`. A ReadWriteOnce claim mounted by more than one concurrent
  consumer is the canonical case.
- `AccessMode`: `read_write_once`, `read_only_many`, `read_write_many`,
  `read_write_once_pod`, `unknown`.

### 5.6 Policy

- `PolicyRecord`: `id`, `name`, `namespace`, `kind`, `resource`, `severity`,
  `result`, `message`, `observed_at`, `details`.
- `PolicyKind`: `istio`, `kyverno`, `flagger`, `other`.
- `PolicyResult`: `pass`, `fail`, `warn`, `error`, `skip`.
- `ValidationResult`: `valid`, `mutated_manifest`, `violations`, `warnings`.

### 5.7 Tap and logs

- `TapHandle`: `id`, `target`, `state`, `created`, `expires`, `max_events`,
  `callback_url`, `filter`.
- `TapEvent`: `id`, `ce_id`, `ce_source`, `ce_type`, `ce_subject`, `ce_time`,
  `headers`, `payload`, `received_at`.
- `LogLine`: `namespace`, `pod`, `container`, `timestamp`, `message`, `stream`
  (`stdout` | `stderr`).

### 5.8 Daemon and configuration

- `DaemonStatus`: `version`, `schema_version`, `context`, `generation`,
  `watchers`, `uptime_secs`, `actions_enabled`, `tap_count`, `started_at`,
  `cache_enabled`, `last_resync`.
- `WatcherStatus`: `gvk`, `resource_count`, `healthy`, `last_event`.
- `SafeConfig`: `daemon`, `discovery`, `watch`, `cache`, `tap`, `ui`, `actions`.
  It contains only the values from `config.get` that are safe to expose and
  never contains a bearer token, kubeconfig, Secret value, or callback token.

---

## 6. Redaction rules

The protocol is designed so a raw secret cannot reach a serialized payload:

1. Secret `data` and `stringData` are removed immediately after link candidates
   are extracted. ConfigMap keys are retained, values are not.
2. `SecretValue` in `knect_proto::redaction` implements `Debug` and `Display`
   with `[REDACTED]`; it implements neither `Serialize` nor `Deserialize`.
3. Link evidence stores `value_redacted` only. The raw value never enters
   `LinkRecord`, `Evidence`, `warning.message`, or `value_preview`.
4. `SafeConfig` is a separate type from the internal config; there is no field
   for a token, kubeconfig path content, or secret.
5. Logs, CloudEvent headers, and mutation previews pass through the redactor
   before becoming `LogLine`, `TapEvent`, or `FieldOwnership`.
6. If redaction cannot be guaranteed, the daemon fails closed with
   `redaction_violation` (`-32010`) instead of returning the object.
7. Tests assert a sentinel secret never appears in any serialized protocol
   payload or contract fixture.

---

## 7. Determinism and ordering

- Maps are sorted by key (`BTreeMap` in Rust) for stable output.
- Graph nodes are ordered by `ResourceId`; edges by edge `id`.
- `TopologyStats.categories` is sorted by category name.
- Timestamps are RFC 3339 UTC strings, for example
  `2026-10-31T12:05:00Z`.
- `generation` is monotonic for the lifetime of a daemon context. A cached
  topology keeps its generation; a resync increments it.
- `seq` is monotonic across the SSE ring and the JSONL subscription stream.

---

## 8. Versioning and compatibility

- `SCHEMA_VERSION = 1` is carried in `Topology.schema_version` and
  `DaemonStatus.schema_version`.
- A client that receives a different schema version reports
  `schema version mismatch` (`-32602`).
- Additive optional fields are allowed within v1. New enum values that older
  clients may not know are decoded as `unknown` where the schema defines it.
- Method names, event names, error codes, field names, and enum tags are
  frozen for v1.
