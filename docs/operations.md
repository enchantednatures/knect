# Operations

## Process model

`knectd` runs one supervised process with:

- a dynamic discovery pass over installed APIs
- one watcher per discovered resource, restarted with bounded backoff
- an ingest/sanitize stage and a generation-counted `ResourceStore`
- a debounced correlation pipeline
- a `petgraph` engine and in-memory topology projection
- a redb snapshot cache per kube-context
- HTTP JSON-RPC, SSE, and UDS JSONL transports
- an optional token-guarded tap callback listener

Shutdown is graceful: watchers and taps are cancelled, pending work is drained,
the cache is flushed, and the Unix socket is removed.

## Health and status

```sh
curl -s http://127.0.0.1:7420/healthz
curl -s -X POST http://127.0.0.1:7420/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"daemon.status","params":{}}'
```

`daemon.status` reports context, generation, watcher health, uptime, action/tap
state, and whether the snapshot cache is enabled.

## Cache

The daemon writes `<cache_dir>/state.db` and takes an exclusive
`state.db.lock`. If the lock is held by another process, the daemon logs a
warning and continues without persistence. Corrupt or schema-mismatched
snapshots are ignored and overwritten on the next recompute.

## Topology and attribution

- `graph.get_topology` returns deterministic nodes/edges and stats.
- `graph.get_node_details` returns redacted YAML, links, policies, storage
  records, managedFields attribution, and warnings.
- `mutations.get_diff` returns the three-way user intent / Knative defaults /
  policy-mesh diff with YAML line ownership.
- `policy.validate_manifest` performs a server-side `dryRun=All`; it never
  mutates the cluster.

## Actions

Actions are disabled by default. Enable both gates deliberately:

```toml
[daemon]
allow_actions = true
[actions]
enabled = true
```

`actions.set_traffic_split` validates revision ownership and percentages;
`actions.flagger_control` promotes/aborts a Canary. The Helm chart grants only
the minimum namespace-scoped write permissions when `actions.enabled` is true.

## Taps and logs

`tap.start` creates an ephemeral, labelled Trigger targeting the daemon's
token-guarded callback. Taps expire by TTL, are removed by `tap.stop`, and are
cleaned up on daemon shutdown and startup GC. `logs.subscribe` streams pod logs
as `log.line` SSE events; `logs.stop` cancels one subscription.

## Kubernetes permissions

The observer needs `get/list/watch` over the Knative, Eventing, Istio,
Kyverno, Flagger, operator, storage, and core resource groups it correlates.
It does not need Secrets by default. Read-only RBAC is the chart default; the
actions Role is opt-in.

## Troubleshooting

- **Daemon starts with empty topology**: verify the context and RBAC; check
  `daemon.status.watchers` and logs for discovery/watch errors.
- **Cache warning**: another daemon holds `state.db.lock`, or the cache is
  corrupt; remove it only while the daemon is stopped.
- **SSE disconnects**: clients should send `Last-Event-ID`; outside the
  replay ring the daemon sends a fresh `snapshot`.
- **Tap fails**: verify `tap.enabled`, the callback Service/port, and that the
  target Broker/Trigger exists. Check for orphaned `knect.dev/managed-by=knect`
  Triggers if a tap was interrupted.
- **Actions disabled**: set both action gates. `policy.validate_manifest` is a
  dry-run and does not require the action gates.
