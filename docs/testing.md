# Testing

## Fast gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

The Rust suite covers:

- `knect-proto`: JSON round-trips, golden contracts, schema stability, redaction
- `knect-core`: config precedence, store indexes, discovery/watch behavior,
  sanitization, graph correlation, links, operators, storage, policy,
  managedFields attribution, actions, tap lifecycle, redb cache
- `knectd`: RPC envelopes, UDS HTTP/JSONL, SSE replay, node details, fixture
  runtime, tap callback, action guards
- `knect-tui`: state/keys, render snapshots, CLI, real PTY dashboard
- `knect-e2e`: harness, scenario registry, spawned fixture daemon

Run the 2k-node/5k-edge cache and frame-budget probes explicitly:

```sh
cargo test -p knect-core --test cache_cold_start --release -- --nocapture
cargo test -p knect-tui --test frame_budget -- --ignored --nocapture
```

## WebUI

```sh
cd webui
npm ci
npm run check
npm test -- --run
npm run build
npm run test:e2e   # Playwright; requires a Chromium with system deps
```

Playwright starts `e2e/mock-server.mjs`, exercises desktop and mobile layouts,
opens the inspector/mutation diff/tap, and checks the RPC failure state.
Screenshots are written under `artifacts/`.

## Neovim

```sh
./hack/nvim-test.sh
```

The script caches Plenary/Nui/Telescope under
`${XDG_CACHE_HOME:-$HOME/.cache}/knect-nvim-deps`, then runs the headless
Plenary suite. It covers the UDS JSONL client, HTTP fallback, pickers, diff,
tap, topology, commands, and YAML extmarks/diagnostics. If a fixture daemon is
available at `/tmp/knectd-fixture.sock`, the suite also exercises it.

## Kind E2E

```sh
./hack/setup-kind-e2e.sh
cargo run -p knect-e2e --locked -- --all
./hack/teardown-kind-e2e.sh
```

The harness uses `/tmp/knect-e2e-kubeconfig` and context `kind-knect-e2e`. It
never reads or modifies the user's default kubeconfig. The full stack includes
Istio, Knative Serving/Eventing, Kafka, RabbitMQ, Kyverno, Flagger, CNPG,
Redis, Redpanda, and Rook CRDs.

Live probes that are normally ignored:

```sh
KUBECONFIG=/tmp/knect-e2e-kubeconfig KNECT_LIVE_CONTEXT=kind-knect-e2e \
  cargo test -p knect-core --test live_discovery --test live_watch -- --ignored --nocapture
```

## Fixture daemon

```sh
./target/debug/knectd --fixture testdata/fixtures \
  --http-addr 127.0.0.1:17420 --socket-path /tmp/knectd-fixture.sock
```

Fixture mode never constructs a Kubernetes client. The fixture topology covers
Serving, Eventing, links, operators, storage, policy, mutations, actions, and
tap scenarios.

## Artifacts

- TUI PTY captures: `target/pty-artifacts/`
- WebUI screenshots/traces: `artifacts/`
- E2E logs: `artifacts/e2e-*.txt`
