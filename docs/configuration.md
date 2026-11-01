# Configuration

`knectd` loads configuration in this order, with later sources overriding
earlier ones:

1. built-in defaults
2. `~/.config/knect/config.toml`
3. `KNECT_*` environment variables
4. CLI flags

A missing config file is not an error. Unknown TOML keys are rejected so typos
surface at startup instead of being silently ignored.

## Example

```toml
[daemon]
socket_path = "/tmp/knectd.sock"
http_addr = "127.0.0.1:7420"
cache_dir = "~/.cache/knect"
context = ""
log = "info"
log_format = "human"
allow_actions = false
allow_remote = false

[discovery]
rediscover_secs = 60
groups = [
  "serving.knative.dev",
  "eventing.knative.dev",
  "sources.knative.dev",
  "messaging.knative.dev",
  "networking.istio.io",
  "security.istio.io",
  "kyverno.io",
  "wgpolicyk8s.io",
  "flagger.app",
  "kafka.strimzi.io",
  "cluster.redpanda.com",
  "rabbitmq.com",
  "dragonflydb.io",
  "redis.redis.opstreelabs.in",
  "postgresql.cnpg.io",
  "storage.k8s.io",
  "ceph.rook.io",
]
namespaces = []

[watch]
debounce_ms = 150
max_objects = 20000
strip_annotations = [
  "kubectl.kubernetes.io/last-applied-configuration",
  "deployment.kubernetes.io/revision",
  "control-plane.alpha.kubernetes.io/leader",
]

[cache]
enabled = true
flush_ms = 500
max_snapshot_bytes = 33554432

[tap]
enabled = true
callback_bind = "0.0.0.0:7421"
callback_host = ""
max_taps = 5
max_events = 1000
ttl_secs = 300

[ui]
fps = 60
theme = "knect-dark"
ascii = false

[actions]
enabled = false
```

## Environment variables

| Variable | Purpose |
|---|---|
| `KNECT_DAEMON_HTTP_ADDR` | HTTP listen address |
| `KNECT_DAEMON_SOCKET_PATH` | Unix socket path |
| `KNECT_DAEMON_CACHE_DIR` / `KNECT_CACHE_DIR` | redb cache directory |
| `KNECT_DAEMON_CONTEXT` | Kubernetes context |
| `KNECT_DAEMON_ALLOW_ACTIONS` | Enable guarded writes |
| `KNECT_ALLOW_REMOTE` | Allow non-loopback HTTP binds |
| `KNECT_LOG` / `KNECT_LOG_FORMAT` | Tracing filter and `human`/`json` format |
| `KNECT_CACHE_ENABLED` | Enable persistent snapshots |
| `KNECT_CACHE_FLUSH_MS` | Cache flush interval |
| `KNECT_TAP_CALLBACK_HOST` | Advertised callback host |
| `KNECT_UI_FPS` | TUI target frames per second |
| `KNECT_ACTIONS_ENABLED` | Action subsystem switch |

## Security defaults

- HTTP binds to loopback unless `daemon.allow_remote` is true.
- The Unix socket is mode `0600`; Linux peers must match the socket owner UID.
- The Helm chart uses a read-only ClusterRole and does not read Secrets unless
  `rbac.readSecrets` is explicitly enabled.
- Guarded writes require both `daemon.allow_actions = true` and
  `actions.enabled = true`. With either false, action RPCs fail closed with
  `-32006 actions_disabled`.
- Secret values are kept only in an in-memory registry for link scanning and
  are never serialized into RPC, SSE, cache, logs, or errors.

## Context and cache

The cache stores one snapshot per Kubernetes context in
`<cache_dir>/state.db`. The daemon takes an exclusive lock on
`state.db.lock`; a second writer fails with a clear cache error. A corrupt or
schema-mismatched snapshot is ignored and replaced on the next recompute.
