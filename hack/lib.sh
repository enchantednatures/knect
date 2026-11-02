#!/usr/bin/env bash
# Shared helpers for the knect Kind E2E harness.
#
# Safety invariants enforced here:
#   * every kubectl/kind invocation points at the isolated kubeconfig
#   * mutating scripts must prove the current context is kind-knect-e2e
#   * the user's default kubeconfig (~/.kube/config) is never read or written
#
# shellcheck shell=bash

set -Eeuo pipefail

KNECT_HACK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
KNECT_REPO_ROOT="$(cd -- "${KNECT_HACK_DIR}/.." && pwd)"

# Never inherit KUBECONFIG from the caller: an inherited admin@talos context is
# exactly the failure mode this harness exists to prevent.
export KUBECONFIG="${KNECT_E2E_KUBECONFIG:-/tmp/knect-e2e-kubeconfig}"

export KIND_CLUSTER_NAME="${KIND_CLUSTER_NAME:-knect-e2e}"
export KIND_CONTEXT="${KIND_CONTEXT:-kind-${KIND_CLUSTER_NAME}}"
export KIND_NODE_IMAGE="${KIND_NODE_IMAGE:-kindest/node:v1.35.0}"
export KIND_CONFIG_FILE="${KNECT_HACK_DIR}/kind-config.yaml"
export KNECT_E2E_CACHE_DIR="${KNECT_E2E_CACHE_DIR:-/tmp/knect-e2e-cache}"
export KNECT_E2E_NAMESPACE="${KNECT_E2E_NAMESPACE:-knect-e2e}"

if [[ -t 1 ]]; then
	KNECT_C_RESET=$'\033[0m'
	KNECT_C_INFO=$'\033[1;34m'
	KNECT_C_WARN=$'\033[1;33m'
	KNECT_C_ERROR=$'\033[1;31m'
	KNECT_C_OK=$'\033[1;32m'
else
	KNECT_C_RESET=""
	KNECT_C_INFO=""
	KNECT_C_WARN=""
	KNECT_C_ERROR=""
	KNECT_C_OK=""
fi

knect_ts() { date -u '+%Y-%m-%dT%H:%M:%SZ'; }

log() { printf '%s %s[%s]%s %s\n' "$(knect_ts)" "${KNECT_C_INFO}" "knect-e2e" "${KNECT_C_RESET}" "$*"; }
ok() { printf '%s %s[%s]%s %s\n' "$(knect_ts)" "${KNECT_C_OK}" "knect-e2e" "${KNECT_C_RESET}" "$*"; }
warn() { printf '%s %s[%s]%s %s\n' "$(knect_ts)" "${KNECT_C_WARN}" "knect-e2e" "${KNECT_C_RESET}" "$*" >&2; }
err() { printf '%s %s[%s]%s %s\n' "$(knect_ts)" "${KNECT_C_ERROR}" "knect-e2e" "${KNECT_C_RESET}" "$*" >&2; }
die() {
	err "$*"
	exit 1
}

require_cmds() {
	local missing=0 cmd
	for cmd in "$@"; do
		if ! command -v "$cmd" >/dev/null 2>&1; then
			err "required command not found: $cmd"
			missing=1
		fi
	done
	[[ "$missing" -eq 0 ]] || exit 1
}

ensure_cache_dir() {
	mkdir -p "${KNECT_E2E_CACHE_DIR}"
}

kubectl_e2e() {
	kubectl --kubeconfig "${KUBECONFIG}" "$@"
}

kind_e2e() {
	kind --kubeconfig "${KUBECONFIG}" "$@"
}

# Refuse to touch anything that is not the disposable Kind cluster.
assert_e2e_context() {
	local current
	current="$(kubectl --kubeconfig "${KUBECONFIG}" config current-context 2>/dev/null || true)"
	if [[ "${current}" != "${KIND_CONTEXT}" ]]; then
		err "refusing to mutate cluster: current context is '${current:-<none>}', expected '${KIND_CONTEXT}'"
		err "kubeconfig: ${KUBECONFIG}"
		exit 2
	fi
}

cluster_exists() {
	kind get clusters 2>/dev/null | grep -Fxq "${KIND_CLUSTER_NAME}"
}

timed() {
	local label="$1"
	shift
	local started rc
	started="${SECONDS}"
	log "START ${label}"
	if "$@"; then
		rc=0
	else
		rc=$?
	fi
	if [[ "${rc}" -eq 0 ]]; then
		ok "DONE  ${label} ($((SECONDS - started))s)"
	else
		err "FAIL  ${label} ($((SECONDS - started))s, rc=${rc})"
	fi
	return "${rc}"
}

download() {
	local url="$1"
	local dest="$2"
	if [[ -s "${dest}" ]]; then
		return 0
	fi
	ensure_cache_dir
	log "download ${url}"
	curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 20 -o "${dest}.tmp" "${url}"
	mv "${dest}.tmp" "${dest}"
}

apply_url() {
	local url="$1"
	local name
	name="$(basename "${url}")"
	download "${url}" "${KNECT_E2E_CACHE_DIR}/${name}"
	kubectl_e2e apply --server-side --field-manager=knect-e2e -f "${KNECT_E2E_CACHE_DIR}/${name}"
}

apply_url_client() {
	local url="$1"
	local name
	name="$(basename "${url}")"
	download "${url}" "${KNECT_E2E_CACHE_DIR}/${name}"
	kubectl_e2e apply -f "${KNECT_E2E_CACHE_DIR}/${name}"
}

apply_dir() {
	local dir="$1"
	[[ -d "${dir}" ]] || die "fixture directory not found: ${dir}"
	local files=("${dir}"/*.yaml)
	[[ -e "${files[0]}" ]] || {
		warn "no YAML files in ${dir}"
		return 0
	}
	local file rc=0
	for file in "${files[@]}"; do
		if ! kubectl_e2e apply -f "${file}"; then
			err "failed to apply fixture ${file}"
			rc=1
		fi
	done
	return "${rc}"
}

wait_for_crd() {
	local timeout="${1:-300s}"
	shift
	local crd
	for crd in "$@"; do
		kubectl_e2e wait --for=condition=Established --timeout="${timeout}" "crd/${crd}"
	done
}

wait_for_rollout() {
	local namespace="$1"
	local kind_name="$2"
	local timeout="${3:-300s}"
	kubectl_e2e -n "${namespace}" rollout status "${kind_name}" --timeout="${timeout}"
}

wait_for_deployment() {
	wait_for_rollout "$1" "deployment/$2" "${3:-300s}"
}

wait_for_statefulset() {
	wait_for_rollout "$1" "statefulset/$2" "${3:-300s}"
}

wait_for_daemonset() {
	wait_for_rollout "$1" "daemonset/$2" "${3:-300s}"
}

# Keep control-plane CPU requests tiny so the full fixture matrix fits on a
# small CI runner. This changes the pod template, so call it before waiting for
# a rollout rather than after. Missing workload kinds are ignored.
relax_cpu_requests() {
	local namespace="$1"
	local kind pods
	for kind in deployment statefulset daemonset; do
		kubectl_e2e -n "${namespace}" set resources "${kind}" --all \
			--requests=cpu=10m >/dev/null 2>&1 || true
	done

	# A rolling update deadlocks when the old pods still hold the runner's
	# scarce CPU: the new low-request pods cannot schedule, so the old ones are
	# never retired. Delete only pods whose requests are still above the cap;
	# controllers recreate them from the patched templates.
	pods="$(
		kubectl_e2e -n "${namespace}" get pods -o json 2>/dev/null |
			python3 -c '
import json, sys

def cpu(value):
    text = str(value or "0")
    try:
        if text.endswith("m"):
            return float(text[:-1]) / 1000
        return float(text)
    except ValueError:
        return 0

data = json.load(sys.stdin)
for pod in data.get("items", []):
    meta = pod.get("metadata", {})
    if meta.get("deletionTimestamp"):
        continue
    spec = pod.get("spec", {})
    containers = spec.get("containers", []) + spec.get("initContainers", [])
    requests = [
        cpu(c.get("resources", {}).get("requests", {}).get("cpu"))
        for c in containers
    ]
    if any(request > 0.015 for request in requests):
        print(meta.get("name", ""))
' 2>/dev/null || true
	)"
	if [[ -n "${pods}" ]]; then
		# Word splitting is intentional: one pod name per line.
		# shellcheck disable=SC2086
		kubectl_e2e -n "${namespace}" delete pod ${pods} \
			--ignore-not-found --wait=false >/dev/null 2>&1 || true
	fi
}

# wait_ready.sh is intentionally a standalone script so humans and CI can use
# it directly. This helper gives installers the same failure diagnostics.
wait_ready() {
	"${KNECT_HACK_DIR}/wait-ready.sh" "$@"
}

find_istioctl() {
	local version="$1"
	local cached="${KNECT_E2E_CACHE_DIR}/istioctl-${version}/istioctl"
	if [[ -x "${cached}" ]]; then
		printf '%s\n' "${cached}"
		return 0
	fi
	if command -v istioctl >/dev/null 2>&1; then
		if istioctl version --remote=false 2>/dev/null | grep -q "client version: ${version}"; then
			command -v istioctl
			return 0
		fi
	fi
	return 1
}

install_istioctl() {
	local version="$1"
	local existing
	if existing="$(find_istioctl "${version}")"; then
		printf '%s\n' "${existing}"
		return 0
	fi

	ensure_cache_dir
	local arch tarball url
	case "$(uname -m)" in
	x86_64 | amd64) arch="amd64" ;;
	aarch64 | arm64) arch="arm64" ;;
	*) die "unsupported architecture for istioctl: $(uname -m)" ;;
	esac
	tarball="${KNECT_E2E_CACHE_DIR}/istioctl-${version}-linux-${arch}.tar.gz"
	url="https://github.com/istio/istio/releases/download/${version}/istioctl-${version}-linux-${arch}.tar.gz"
	download "${url}" "${tarball}" >&2
	mkdir -p "${KNECT_E2E_CACHE_DIR}/istioctl-${version}"
	tar -xzf "${tarball}" -C "${KNECT_E2E_CACHE_DIR}/istioctl-${version}"
	chmod +x "${KNECT_E2E_CACHE_DIR}/istioctl-${version}/istioctl"
	printf '%s\n' "${KNECT_E2E_CACHE_DIR}/istioctl-${version}/istioctl"
}
