# knect

`knect` is a Rust, headless-first observability, topology, policy-attribution,
and control plane for Knative Serving/Eventing and its surrounding
infrastructure: Istio, Flagger, Kyverno, Kafka/Redpanda, RabbitMQ,
Redis/Dragonfly, Postgres/MySQL, and Rook-Ceph.

It separates cluster state correlation from presentation:

- **`knectd`** — the stateful daemon. It watches Kubernetes resources, builds a
  `petgraph` topology, attributes YAML mutations through `managedFields`,
  overlays mesh/policy state, caches snapshots in `redb`, and exposes a
  JSON-RPC 2.0 + SSE API over a Unix socket and HTTP.
- **`knect`** — the Ratatui terminal client and CLI. It can auto-spawn a local
  daemon, connect to a remote daemon, inspect topology and mutations, and
  integrate with `k9s`.
- **`knect.nvim`** — the Neovim plugin. It provides a Lua JSON-RPC client,
  topology/pickers/diff/tap views, and live YAML inlay hints.
- **`knect-web`** — a Svelte 5 + SvelteFlow topology UI embedded into `knectd`
  with `rust-embed`.

## Quick start

Build and test the workspace:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

Run the daemon against the current kubeconfig:

```sh
./target/debug/knectd
```

Run it without a cluster using the checked-in fixtures:

```sh
./target/debug/knectd --fixture testdata/fixtures
```

The daemon listens on `127.0.0.1:7420` and `/tmp/knectd.sock` by default.
Open `http://127.0.0.1:7420/` for the embedded WebUI.

Connect the TUI:

```sh
./target/debug/knect --socket /tmp/knectd.sock
```

Install the Neovim plugin by adding `nvim/` to `runtimepath`, then run
`:Knect`, `:KnectServices`, `:KnectBrokers`, `:KnectDiff`, or `:KnectTap`.
Optional integrations: Nui.nvim, Telescope.nvim, and Plenary.nvim (tests).

## Development

```sh
make ci          # fmt + clippy + workspace tests
make web         # build the Svelte WebUI
make web-test    # Vitest
make web-e2e     # Playwright (requires a Playwright browser + system deps)
make nvim-test   # headless Plenary suite
make e2e         # run the E2E harness against the Kind cluster
make helm        # lint and template the Helm chart
```

The full-stack Kind E2E environment is created with:

```sh
./hack/setup-kind-e2e.sh
./hack/teardown-kind-e2e.sh
```

It uses an isolated kubeconfig at `/tmp/knect-e2e-kubeconfig` and never
touches the user's default kubeconfig.

## Configuration

Configuration is read from `~/.config/knect/config.toml`, then `KNECT_*`
environment variables, then CLI flags. See
[docs/configuration.md](docs/configuration.md). The read-only defaults are
intentional: guarded write actions require both
`daemon.allow_actions = true` and `actions.enabled = true`.

## Documentation

- [Architecture](docs/architecture.md)
- [Protocol](docs/protocol.md)
- [Configuration](docs/configuration.md)
- [Testing](docs/testing.md)
- [Operations](docs/operations.md)
- [Release](docs/release.md)

## Status

Unreleased `v1.0.0` release candidate. The daemon, cache,
topology/correlation engines, TUI, WebUI, `knect.nvim`, container, Helm chart,
and signed release pipeline are implemented and tested.

Known limitation: a cold full-stack Kind setup currently takes about **13
minutes**, dominated by Knative install and readiness waits. The original
3-minute E2E target requires a pre-baked Kind node image; the scripts and
resource limits are already in place for that optimization.
