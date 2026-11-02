#!/usr/bin/env bash
# Create the disposable knect-e2e Kind cluster and install the full component
# matrix required by issue #2: cert-manager, Istio, Knative Serving/Eventing,
# Kafka Broker, RabbitMQ Broker, Kyverno, Flagger, and backing services.
#
# Every kubectl/kind call is pinned to /tmp/knect-e2e-kubeconfig and the
# kind-knect-e2e context. The user's default kubeconfig is never touched.
#
# Usage:
#   hack/setup-kind-e2e.sh [--no-preload] [--no-wait]

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

PRELOAD="${KNECT_PRELOAD_IMAGES:-1}"
WAIT_AT_END=1
CURRENT_PHASE="init"
SETUP_STARTED="${SECONDS}"

usage() {
	sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--no-preload)
		PRELOAD=0
		shift
		;;
	--no-wait)
		WAIT_AT_END=0
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

on_error() {
	local rc=$?
	err "setup failed during phase '${CURRENT_PHASE}' (rc=${rc})"
	err "the cluster is left in place for debugging; run hack/teardown-kind-e2e.sh to remove it"
	if [[ -f "${KUBECONFIG}" ]]; then
		kubectl_e2e get pods -A -o wide >&2 || true
	fi
	exit "${rc}"
}
trap on_error ERR

run_phase() {
	local label="$1"
	shift
	CURRENT_PHASE="${label}"
	timed "${label}" "$@"
}

preflight() {
	require_cmds docker kind kubectl helm curl
	docker info >/dev/null 2>&1 || die "Docker daemon is not reachable"
	kind version
	kubectl version --client
	helm version --short
}

create_cluster() {
	CURRENT_PHASE="cluster"
	if cluster_exists; then
		log "Kind cluster '${KIND_CLUSTER_NAME}' already exists; reusing it"
	else
		log "creating Kind cluster '${KIND_CLUSTER_NAME}' (${KIND_NODE_IMAGE})"
		kind_e2e create cluster \
			--name "${KIND_CLUSTER_NAME}" \
			--config "${KIND_CONFIG_FILE}" \
			--wait 240s
	fi

	kind_e2e export kubeconfig --name "${KIND_CLUSTER_NAME}"
	assert_e2e_context
	kubectl_e2e get nodes -o wide

	# Recreate kube-system workload pods with small requests so the fixture
	# matrix fits on the runner; static control-plane pods are left alone.
	relax_cpu_requests kube-system
	wait_ready --namespace kube-system --timeout 300
}

prime_traffic_split_revisions() {
	local svc rc=0
	for svc in order-processor checkout; do
		if ! kubectl_e2e -n knect-e2e patch ksvc "${svc}" --type merge \
			-p '{"spec":{"template":{"metadata":{"annotations":{"knect.dev/primed-revision":"2"}}}}}' >/dev/null; then
			err "failed to prime a second revision for ${svc}"
			rc=1
			continue
		fi
		if ! kubectl_e2e -n knect-e2e wait --for=condition=Ready "revision/${svc}-00002" --timeout=180s; then
			warn "${svc}-00002 was not ready within 180s; the traffic split fixture will remain not Ready"
		fi
	done
	return "${rc}"
}

install_backing_then_fixtures() {
	local dir rc=0
	for dir in links infra storage operators serving eventing mutations policy actions tap; do
		apply_dir "${KNECT_HACK_DIR}/fixtures/${dir}" || rc=1
	done
	prime_traffic_split_revisions || rc=1
	return "${rc}"
}

log "knect Kind E2E setup"
log "kubeconfig=${KUBECONFIG} cluster=${KIND_CLUSTER_NAME} context=${KIND_CONTEXT}"

run_phase "preflight" preflight
create_cluster

if [[ "${PRELOAD}" == "1" ]]; then
	run_phase "preload images" bash "${KNECT_HACK_DIR}/preload-images.sh"
else
	log "image preload disabled (KNECT_PRELOAD_IMAGES=0)"
fi

run_phase "cert-manager" bash "${KNECT_HACK_DIR}/install-cert-manager.sh"
run_phase "istio" bash "${KNECT_HACK_DIR}/install-istio.sh"
run_phase "knative" bash "${KNECT_HACK_DIR}/install-knative.sh"
run_phase "policy" bash "${KNECT_HACK_DIR}/install-policy.sh"
run_phase "flagger" bash "${KNECT_HACK_DIR}/install-flagger.sh"
run_phase "backing services" bash "${KNECT_HACK_DIR}/install-backing.sh"
run_phase "fixtures" install_backing_then_fixtures

if [[ "${WAIT_AT_END}" == "1" ]]; then
	CURRENT_PHASE="final wait"
	run_phase "final wait" wait_ready --all --timeout "${KNECT_E2E_FINAL_WAIT:-1200}"
fi

CURRENT_PHASE="summary"
kubectl_e2e get nodes -o wide
kubectl_e2e get pods -A -o wide
ok "knect-e2e setup complete in $((SECONDS - SETUP_STARTED))s"
