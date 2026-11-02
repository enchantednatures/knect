#!/usr/bin/env bash
# Wait for workloads in the disposable knect-e2e cluster to become Ready.
#
# Usage:
#   hack/wait-ready.sh [--timeout 600] [--namespace NS | --all]
#                      [--selector k=v] [--skip-deployments]
#
# Exit codes: 0 ready, 1 timeout/not ready, 2 wrong Kubernetes context.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

TIMEOUT="${KNECT_WAIT_TIMEOUT:-600}"
NAMESPACE=""
ALL_NAMESPACES=0
SELECTOR=""
WAIT_DEPLOYMENTS=1

usage() {
	sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--timeout)
		TIMEOUT="$2"
		shift 2
		;;
	--namespace | -n)
		NAMESPACE="$2"
		shift 2
		;;
	--all | -A)
		ALL_NAMESPACES=1
		shift
		;;
	--selector | -l)
		SELECTOR="$2"
		shift 2
		;;
	--skip-deployments)
		WAIT_DEPLOYMENTS=0
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		die "unknown argument: $1"
		;;
	esac
done

# Accept kubectl-style durations as well as plain seconds.
case "${TIMEOUT}" in
*s) TIMEOUT="${TIMEOUT%s}" ;;
*m) TIMEOUT="$(( ${TIMEOUT%m} * 60 ))" ;;
*h) TIMEOUT="$(( ${TIMEOUT%h} * 3600 ))" ;;
esac
[[ "${TIMEOUT}" =~ ^[0-9]+$ ]] || die "invalid timeout: ${TIMEOUT}"

require_cmds kubectl python3
assert_e2e_context

scope_args=()
if [[ "${ALL_NAMESPACES}" -eq 1 || -z "${NAMESPACE}" ]]; then
	scope_args=(-A)
	scope="all namespaces"
else
	scope_args=(-n "${NAMESPACE}")
	scope="namespace ${NAMESPACE}"
fi
if [[ -n "${SELECTOR}" ]]; then
	scope_args+=(-l "${SELECTOR}")
fi

deadline=$((SECONDS + TIMEOUT))
workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT
last_summary=""

dump_json() {
	local out="$1"
	shift
	if ! kubectl_e2e "$@" -o json >"${out}" 2>/dev/null; then
		printf '{"items":[]}' >"${out}"
	fi
}

while :; do
	dump_json "${workdir}/pods.json" get pods "${scope_args[@]}"
	if [[ "${WAIT_DEPLOYMENTS}" -eq 1 ]]; then
		dump_json "${workdir}/deployments.json" get deployments "${scope_args[@]}"
		dump_json "${workdir}/statefulsets.json" get statefulsets "${scope_args[@]}"
		dump_json "${workdir}/daemonsets.json" get daemonsets "${scope_args[@]}"
		dump_json "${workdir}/jobs.json" get jobs "${scope_args[@]}"
	else
		for f in deployments statefulsets daemonsets jobs; do
			printf '{"items":[]}' >"${workdir}/${f}.json"
		done
	fi

	summary="$(
		python3 - "${workdir}/pods.json" "${workdir}/deployments.json" "${workdir}/statefulsets.json" "${workdir}/daemonsets.json" "${workdir}/jobs.json" <<'PY'
import json, sys

def load(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh).get("items", [])
    except (OSError, json.JSONDecodeError) as exc:
        return [{"__parse_error__": str(exc)}]

pods, deploys, statefuls, daemons, jobs = (load(path) for path in sys.argv[1:6])

problems = []

for pod in pods:
    if "__parse_error__" in pod:
        problems.append(f"pods: {pod['__parse_error__']}")
        continue
    meta = pod.get("metadata", {})
    status = pod.get("status", {})
    name = f"{meta.get('namespace','?')}/{meta.get('name','?')}"
    if meta.get("deletionTimestamp"):
        continue
    phase = status.get("phase", "Unknown")
    if phase == "Succeeded":
        continue
    owners = meta.get("ownerReferences") or []
    job_owned = any(owner.get("kind") == "Job" for owner in owners)
    if phase == "Failed":
        if not job_owned:
            problems.append(f"pod {name} phase={phase}")
        continue
    if phase == "Unknown":
        problems.append(f"pod {name} phase={phase}")
        continue
    statuses = status.get("containerStatuses") or []
    init_statuses = status.get("initContainerStatuses") or []
    if phase != "Running":
        problems.append(f"pod {name} phase={phase}")
        continue
    not_ready = [c.get("name", "?") for c in statuses if not c.get("ready")]
    not_ready_init = [c.get("name", "?") for c in init_statuses if not c.get("ready")]
    if not_ready or not_ready_init:
        detail = ",".join(not_ready + not_ready_init)
        problems.append(f"pod {name} containers not ready: {detail}")
        continue
    if not statuses and not init_statuses:
        problems.append(f"pod {name} has no container status yet")
        continue
    for c in statuses:
        waiting = (c.get("state") or {}).get("waiting") or {}
        if waiting.get("reason") in ("CrashLoopBackOff", "ImagePullBackOff", "ErrImagePull"):
            problems.append(f"pod {name} container {c.get('name','?')}={waiting['reason']}")

for kind, items in (("deployment", deploys), ("statefulset", statefuls)):
    for obj in items:
        if "__parse_error__" in obj:
            problems.append(f"{kind}s: {obj['__parse_error__']}")
            continue
        meta = obj.get("metadata", {})
        status = obj.get("status", {})
        spec = obj.get("spec", {})
        name = f"{meta.get('namespace','?')}/{meta.get('name','?')}"
        desired = int(spec.get("replicas", 0) or 0)
        available = int(status.get("availableReplicas", 0) or 0)
        if available < desired:
            problems.append(f"{kind} {name} available={available}/{desired}")

for obj in daemons:
    if "__parse_error__" in obj:
        problems.append(f"daemonsets: {obj['__parse_error__']}")
        continue
    meta = obj.get("metadata", {})
    status = obj.get("status", {})
    name = f"{meta.get('namespace','?')}/{meta.get('name','?')}"
    desired = int(status.get("desiredNumberScheduled", 0) or 0)
    ready = int(status.get("numberReady", 0) or 0)
    if ready < desired:
        problems.append(f"daemonset {name} ready={ready}/{desired}")

for job in jobs:
    if "__parse_error__" in job:
        problems.append(f"jobs: {job['__parse_error__']}")
        continue
    meta = job.get("metadata", {})
    status = job.get("status", {})
    name = f"{meta.get('namespace','?')}/{meta.get('name','?')}"
    conditions = status.get("conditions") or []
    if any(c.get("type") == "Complete" and c.get("status") == "True" for c in conditions):
        continue
    failed = int(status.get("failed", 0) or 0)
    active = int(status.get("active", 0) or 0)
    if failed > 0 and active == 0:
        problems.append(f"job {name} failed={failed}, active={active}")

if problems:
    print("NOT_READY")
    for problem in problems:
        print(problem)
else:
    print("READY")
PY
	)"
	if [[ "${summary}" == "READY" ]]; then
		ok "all workloads ready in ${scope}"
		exit 0
	fi

	if [[ "${summary}" != "${last_summary}" ]]; then
		printf '%s\n' "${summary}" | head -n 25 >&2
		last_summary="${summary}"
	fi

	if [[ "${SECONDS}" -ge "${deadline}" ]]; then
		err "timed out after ${TIMEOUT}s waiting for workloads in ${scope}"
		err "--- pods ---"
		kubectl_e2e get pods "${scope_args[@]}" -o wide || true
		err "--- not ready ---"
		printf '%s\n' "${summary}" >&2
		err "--- recent events ---"
		kubectl_e2e get events "${scope_args[@]}" --sort-by=.lastTimestamp 2>/dev/null | tail -n 60 || true
		err "--- pod descriptions ---"
		kubectl_e2e describe pods "${scope_args[@]}" 2>/dev/null | tail -n 200 || true
		exit 1
	fi
	sleep 5
done
