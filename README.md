# knect

knect is a Rust, headless-first observability and control-plane toolchain for
Knative, Istio, Flagger, and Kyverno: it collects signals and builds a live
topology of the mesh, attributes policy and traffic behavior back to concrete
workloads and resources, and exposes that model through a headless daemon and
purpose-built clients.

## Architecture

- **knectd** — the headless daemon: watches clusters, gathers observability
  signals, builds topology, performs policy attribution, and serves the control
  plane API.
- **knect** — the terminal UI (TUI) client for interactive inspection of
  topology, signals, and policy attribution.
- **knect.nvim** — the Neovim frontend for working with the same control plane
  from inside the editor.
- **knect-web** — the web UI for browsing the topology and related data.

## Status

`v1.0.0` in progress. This repository currently contains the project skeleton —
toolchain, lint, CI, and documentation scaffolding. Features are not yet
complete and nothing here should be treated as a finished capability.

## Build

```sh
make ci
```

Placeholder: build instructions will be finalized as the workspace is added.

## Test

```sh
make test
```

Placeholder: test instructions will be finalized as crates and test harnesses
are added.
