# knect architecture and frozen interfaces

Status: v1.1.0. The protocol in `docs/protocol.md` and
`crates/knect-proto` is
frozen. The daemon runtime, redb cache, discovery and watch supervisors,
Serving/Eventing topology, service-link/operator/storage/policy/mutation
attribution, Ratatui TUI, embedded SvelteFlow WebUI, `knect.nvim`, container,
Helm chart, and release pipeline are implemented.

## 0. Implemented runtime

- `knectd` discovers installed APIs, watches 90+ GVKs, sanitizes objects,
  maintains a redb snapshot per context, recomputes on a 150 ms debounce, and
  serves JSON-RPC 2.0 over HTTP/UDS plus SSE and UDS JSONL event streams.
- The correlation pipeline is fixed at
  `serving -> eventing -> links -> operators -> storage -> policy -> mutations`.
- `graph.get_node_details` projects the node, redacted YAML, links, policies,
  storage records, managedFields attribution, and warnings.
- Fixture mode loads `testdata/fixtures` and serves the real topology without
  constructing a Kubernetes client.
- Tests: 163+ core library tests, exact Serving/Eventing/link/operator/storage/
  policy/mutation fixtures, 60+ daemon tests, TUI PTY/snapshot tests, WebUI
  unit + Playwright checks, and a Plenary nvim suite.

## 1. Dependency direction

```text
knect-proto  (pure wire contract; no I/O, no kube, no runtime)
     ^
     |
knect-core   (config, Kubernetes boundary, store, graph, correlation registry)
     ^                ^
     |                |
knectd          knect-tui / knect-e2e
```

- `knect-proto` must not depend on any other workspace crate.
- `knect-core` depends only on `knect-proto` and pinned external libraries.
- `knectd` depends on `knect-core` and `knect-proto`.
- `knect-tui` depends on `knect-proto` (plus `knect-core` only if a later task
  needs shared helpers).
- `knect-e2e` depends on `knect-proto` and talks to the daemon over HTTP or the
  Unix socket, never by linking daemon internals.

## 2. Crate module map

### `crates/knect-core`

| Module | Owns | Foundation state |
|---|---|---|
| `lib.rs` | Re-exports and `version()` | frozen |
| `error.rs` | `CoreError`, `code()`, `retryable()`, `to_rpc_error`, `From<CoreError> for RpcError` | frozen |
| `config.rs` | `Config`, `DaemonConfig`, `DiscoveryConfig`, `WatchConfig`, `CacheConfig`, `TapConfig`, `UiConfig`, `ActionsConfig`, `ConfigOverrides`, loader, `Config::safe()` | frozen |
| `kube/mod.rs` | `ClusterReader`, `DiscoverySnapshot`, `DiscoveredResource`, `DryRunResult`, `LogOptions`, `WatchEvent` | frozen trait |
| `kube/reader.rs` | `KubeReader` over `kube::Client`: discovery, list/get/patch/dry-run, and real watch streaming; log stream still typed `Unsupported` | Task 12 replaces log streaming |
| `kube/discovery.rs` | `DiscoveryManager`, group/verb filtering, missing-group warnings, pure `find` | implemented |
| `kube/watch.rs` | `WatchManager`, typed `WatchUpdate`/`WatchOutcome`, 404 `ResourceGone`, deterministic backoff | implemented |
| `kube/sanitize.rs` | annotation stripping, status cap, secret material extraction, serializable redaction | implemented |
| `kube/fake.rs` | scripted `FakeReader` for discovery/list/get/patch/dry-run/watch tests | implemented |
| `store/mod.rs` | `ResourceStore`, `StoreEvent`, indexes, generation | frozen |
| `store/resource.rs` | `ResourceId` derivation, owner keys, label matching | frozen |
| `graph/mod.rs` | Re-exports `GraphBuilder`, `GraphEngine`, `diff_topologies`, `stats_for`, `edge_id` | API frozen |
| `graph/engine.rs` | `GraphBuilder`/`GraphEngine` over `StableDiGraph` | implemented |
| `graph/patch.rs` | deterministic `GraphPatch` diffing | implemented |
| `graph/stats.rs` | deterministic node/edge/warning/category statistics and truncation flag | implemented |
| `graph/support.rs` | shared node/edge/time/condition/sanitization helpers and stable edge IDs | implemented |
| `correlate/mod.rs` | `CorrelationContext`, `Correlator`, `CorrelationPipeline`, frozen `STAGES` order | frozen registry |
| `correlate/serving.rs` | KService -> Route/Configuration -> Revision -> PA -> Deployment -> Pod, traffic and scale states | implemented |
| `correlate/eventing.rs` | Broker/Trigger/EventType/Source/SinkBinding/Subscription/Channel/Sink, filters, DLS, MT/Kafka/RabbitMQ classes | implemented |
| `correlate/{links,operators,storage,mutations}.rs` | No-op stage stubs | Task 6/7/8/9 replace bodies |
| `correlate/policy/{mod,istio,flagger,kyverno}.rs` | Policy aggregator plus no-op sub-stages | Task 10 replaces bodies |
| `cache/mod.rs` | `SnapshotCache`, `NoopCache` | Task 5 replaces implementation |
| `actions/mod.rs` | `ActionExecutor`, `DisabledActions` | Task 11 replaces implementation |
| `tap/mod.rs` | `TapManager`, `NoopTapManager` | Task 12 replaces implementation |
| `links/mod.rs`, `operators/mod.rs`, `storage/mod.rs`, `mutations/mod.rs`, `policy/mod.rs` | Extension traits and protocol re-exports | later tasks extend |

### `crates/knectd`

| Module | Owns | Foundation state |
|---|---|---|
| `src/main.rs` | Clap flags, logging, runtime + HTTP + UDS supervision, signal handling, socket cleanup | frozen CLI |
| `src/app.rs` | `AppState` (config, topology cell, store, cache/actions/tap traits, generation, timestamps, `EventHub`, watcher/resync state) | API frozen |
| `src/runtime.rs` | `Runtime::start` and `Runtime::run`; fixture startup and cache load/flush | Task 3/5 extend supervision |
| `src/rpc/mod.rs` | `RpcDispatcher`, all 15 method names, envelope validation, implemented methods, typed stubs | method table frozen |
| `src/transport/http.rs` | Router, `POST /rpc`, `GET /events`, `GET /healthz`, static fallback, TCP listener | frozen routes |
| `src/transport/sse.rs` | `EventHub`, 1,000-event replay ring, 15 s heartbeats, frame encoding | frozen |
| `src/transport/uds.rs` | Mode-0600 UDS, stale-socket replacement, HTTP/1.1 and JSONL first-byte peek, cleanup guard | Task 5 adds peer UID check |
| `src/transport/jsonl.rs` | Newline-delimited request/response helper; subscriptions pending | Task 5 completes streaming |
| `src/fixture.rs` | `FixtureMode`, recursive JSON/YAML fixture loading, empty fixture startup | frozen |
| `src/webui.rs` | `rust-embed` placeholder SPA fallback | Task 16 replaces assets/CSP |
| `src/logging.rs` | tracing human/JSON init and secret-key redaction helper | frozen |
| `src/error.rs` | `DaemonError` typed daemon failures | frozen |

### `crates/knect-tui`

`cli.rs` and `cli/{graph,diff,tap,k9s,install}.rs` freeze the clap surface.
`client/mod.rs` freezes `RpcClient`, `ClientError`, `Endpoint`, and
`resolve_endpoint`. `app/{state,event,keys,actions}.rs` freeze the state shell.
`ui/{mod,layout,tree,graph,inspector,logs,theme,help}.rs` compile to a single
placeholder frame; Task 13 replaces the rendering.

### `crates/knect-e2e`

`harness.rs` freezes HTTP/UDS JSON-RPC, `wait_for`, `assert_topology`,
`assert_event`, and temp path/port helpers. `scenarios/mod.rs` freezes the
registry and `run_all`/`run_scenario`; every scenario module is a pending
skeleton.

## 3. Runtime task graph

```text
main
 ├─ Runtime::start        (fixture load or empty/cached topology; no kube in fixture)
 ├─ Runtime::run          (wait for cancel, flush cache)
 ├─ HTTP listener         (axum router)
 ├─ UDS listener          (HTTP/1.1 or JSONL per connection)
 └─ shutdown signal       (SIGINT/SIGTERM -> CancellationToken -> drain -> remove socket)
```

Tasks 3 and 5 add, behind the same supervisor:

```text
DiscoverySupervisor -> WatchSupervisor -> Ingest -> Recompute -> ArcSwap/SSE
```

`Recompute` runs the correlation pipeline in the frozen order, swaps the
topology, and publishes graph patches. The foundation deliberately does not
fake a cluster in normal mode; the topology stays empty until those tasks land.

## 4. Correlation pipeline order

The registry order is a contract, enforced by
`CorrelationPipeline::from_correlators`:

```text
serving -> eventing -> links -> operators -> storage -> policy -> mutations
```

Every stage receives `&CorrelationContext` (context, generation, store) and
`&mut GraphBuilder`. Adding or reordering a stage requires a foundation change;
later tasks only replace stub bodies.

## 5. Data flow

```text
Kubernetes API
  -> ClusterReader (discover/list/watch/get/patch/dry-run/log)
  -> Sanitizer (strips heavy annotations, caps oversized status, redacts Secrets)
  -> ResourceStore (BTreeMap + GVK/namespace/owner/label indexes, generation)
  -> CorrelationPipeline (pure, fixed order; serving/eventing implemented)
  -> GraphBuilder
  -> GraphEngine (StableDiGraph, stable ResourceId -> NodeIndex)
  -> Topology projection (nodes by ResourceId, edges by id, sorted stats)
  -> AppState topology cell / SSE ring
  -> JSON-RPC clients (TUI, WebUI, nvim) and `GET /events`
```

Fixture mode replaces the first three steps with a directory loader and never
constructs a Kubernetes client.

## 6. Frozen interface contract for later tasks

1. **Protocol**: do not rename or remove `knect-proto` fields, enum tags,
   methods, event names, or error codes. Additive optional fields only.
2. **Errors**: `CoreError` variants and the `code()`/`retryable()` mapping are
   frozen. New behavior must fit an existing variant or be added through the
   orchestrator.
3. **Reader**: `ClusterReader` keeps its hand-desugared object-safe signatures.
   Discovery/watch/get/patch/dry-run are implemented; Task 12 implements
   `log_stream`.
4. **Store**: `upsert` returns `StoreEvent`, every mutation bumps the
   generation, iteration is `ResourceId` order, and removal cleans every index.
5. **Graph**: `GraphBuilder` methods are the only correlator mutation surface.
   `GraphEngine::apply` keeps stable node indices for retained IDs and drops
   edges with missing endpoints with a warning.
6. **Correlators**: every stage is pre-registered. Later tasks must not edit
   `correlate/mod.rs` to add a stage.
7. **RPC**: all 15 methods stay registered. Unknown methods return `-32601`;
   invalid envelopes return `-32600`; invalid params return `-32602`. Later
   tasks replace the typed `INTERNAL_ERROR`/`ACTIONS_DISABLED` stub bodies, not
   the method table or dispatcher shape.
8. **Transports**: `POST /rpc`, `GET /events`, `GET /healthz`, UDS mode `0600`,
   HTTP/1.1 + JSONL framing, SSE `id`/`event`/`data` frame shape, and the
   1,000-event replay ring are frozen.
9. **Clients**: `RpcClient`, endpoint resolution, CLI subcommands, and the E2E
   harness signatures are frozen.
10. **Ownership**: later tasks edit only the files listed for them in the plan;
    new interfaces go through a foundation change.

## 7. Deliberate foundation decisions

- **Topology cell**: the pinned workspace dependency set does not include
  `arc-swap`. `AppState` uses `std::sync::RwLock<Arc<Topology>>`; readers hold
  the lock only to clone the `Arc`, and `topology()` hides the choice. Task 5
  may swap in `ArcSwap` without changing callers.
- **Async traits**: `async-trait` is not in the pinned dependency set either.
  `ClusterReader`, `SnapshotCache`, `ActionExecutor`, and `TapManager` use
  explicit `BoxFuture`/`BoxStream` signatures so they remain object-safe.
- **k8s-openapi version feature**: the pinned workspace uses Kubernetes 1.35
  only, so `knect-core` enables `k8s-openapi/v1_35` directly rather than via a
  dev-dependency. This keeps `cargo check -p knect-core` working for parallel
  tasks that do not build the daemon. A future multi-version build must move
  the feature choice back to the binary crate.
- **Fixture topology**: serving and eventing correlators are real; the daemon
  still needs the Task 5 recompute loop to serve their output through RPC/SSE.
  Fixture mode can already load objects into the store.
- **Discovery**: discovery uses aggregated `kube::discovery::Discovery` with a
  legacy fallback. Watchers use the discovered `ApiResource` plural; the
  synthesized plural in `KubeReader::get`/`patch_merge` remains until a reader
  cache is wired.
- **`min_generation`**: the dispatcher returns the current topology instead of
  blocking. The bounded catch-up wait belongs to the Task 5 recompute loop.
- **Actions**: both `daemon.allow_actions` and `actions.enabled` must be true.
  `--allow-actions` sets both. The foundation executor is always
  `DisabledActions`; Task 11 installs the real executor.
- **Tap/logs/events subscriptions**: stubs return typed errors. HTTP users get
  `GET /events`; JSONL subscription streaming is Task 5.

## 8. Test layers

| Layer | Where | Foundation coverage |
|---|---|---|
| Pure unit | `knect-core` lib tests | config precedence, error mapping, store indexes, discovery filtering, watch mapping/backoff, sanitizer redaction, graph determinism, patch/stats |
| Fixture snapshots | `knect-core` integration tests | exact Serving/Eventing nodes, edges, traffic metadata, scale states, filters |
| Router/transport unit | `knectd` lib tests | dispatcher method table, invalid envelope/params, empty topology, SSE frames, UDS permissions, JSONL round-trip, fixture startup |
| TUI unit | `knect-tui` lib tests | clap parsing, endpoint resolution, key mapping, `TestBackend` placeholder frame |
| Harness unit | `knect-e2e` lib tests | endpoint selection, topology validation, event assertions, scenario registry |
| Real surface | curl/manual + `knect-e2e` | `knectd --fixture` HTTP, UDS, SSE, socket cleanup |
| Fake kube API | implemented | `FakeReader` discovery/list/get/patch/dry-run/watch edge cases |
| Live Kind | Task 2/5+ | discovery/watch probes and `knect-e2e --all` |

## 9. Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --locked
cargo check --workspace --locked
cargo run -p knectd --locked -- --fixture --http-addr 127.0.0.1:17420 --socket-path /tmp/knectd-foundation.sock
cargo run -p knect-tui --locked -- --help
cargo run -p knect-e2e --locked -- --all
```
