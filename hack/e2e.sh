#!/usr/bin/env bash
# Run the knect E2E suite against the disposable Kind cluster.
#
# Usage:
#   hack/e2e.sh [--skip-setup] [--teardown] [-- <extra knect-e2e args>]
#
# By default the cluster is created/installed first, `cargo run -p knect-e2e
# --locked -- --all` is executed, and the cluster is left running for debugging.
# Pass --teardown to remove the Kind cluster when the command exits.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SKIP_SETUP="${KNECT_SKIP_SETUP:-0}"
TEARDOWN="${KNECT_E2E_TEARDOWN:-0}"
E2E_ARGS=(--all)

while [[ $# -gt 0 ]]; do
	case "$1" in
	--skip-setup)
		SKIP_SETUP=1
		shift
		;;
	--teardown)
		TEARDOWN=1
		shift
		;;
	--all)
		E2E_ARGS=(--all)
		shift
		;;
	--)
		shift
		E2E_ARGS=("$@")
		break
		;;
	-h | --help)
		sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*)
		E2E_ARGS+=("$1")
		shift
		;;
	esac
done

if [[ "${TEARDOWN}" == "1" ]]; then
	trap '"${KNECT_HACK_DIR}/teardown-kind-e2e.sh"' EXIT
fi

if [[ "${SKIP_SETUP}" != "1" ]]; then
	bash "${KNECT_HACK_DIR}/setup-kind-e2e.sh"
fi

cluster_exists || die "Kind cluster '${KIND_CLUSTER_NAME}' is not running; run hack/setup-kind-e2e.sh"
assert_e2e_context

[[ -f "${KNECT_REPO_ROOT}/Cargo.toml" ]] ||
	die "Cargo workspace not found at ${KNECT_REPO_ROOT}; the knect-e2e crate is not present yet"

log "running: cargo run -p knect-e2e --locked -- ${E2E_ARGS[*]}"
cd "${KNECT_REPO_ROOT}"
cargo run -p knect-e2e --locked -- "${E2E_ARGS[@]}"
ok "E2E suite completed"
