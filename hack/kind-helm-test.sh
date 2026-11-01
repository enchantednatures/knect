#!/usr/bin/env bash
# Build the knectd image and install charts/knectd into the disposable
# kind-knect-e2e cluster. This is the local counterpart to the docker-helm
# workflow and never touches the caller's default kubeconfig.
#
# The script refuses to run unless the current context is exactly
# kind-knect-e2e. It uninstalls the release on exit but leaves the Kind cluster
# and its images intact.
#
# Usage:
#   hack/kind-helm-test.sh [--skip-build] [--skip-port-forward]
#
# Environment:
#   KNECT_HELM_IMAGE       image reference (default: knectd:helm-local)
#   KNECT_HELM_NAMESPACE   namespace (default: knect-helm-test)
#   KNECT_HELM_RELEASE     Helm release name (default: knectd)
#   KNECT_HELM_PLATFORM    image platform (default: linux/amd64)

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SKIP_BUILD=0
SKIP_PORT_FORWARD=0
while [[ $# -gt 0 ]]; do
	case "$1" in
	--skip-build) SKIP_BUILD=1 ;;
	--skip-port-forward) SKIP_PORT_FORWARD=1 ;;
	-h | --help)
		sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*)
		die "unknown argument: $1"
		;;
	esac
	shift
done

require_cmds docker kind kubectl helm curl

IMAGE="${KNECT_HELM_IMAGE:-knectd:helm-local}"
NAMESPACE="${KNECT_HELM_NAMESPACE:-knect-helm-test}"
RELEASE="${KNECT_HELM_RELEASE:-knectd}"
PLATFORM="${KNECT_HELM_PLATFORM:-linux/amd64}"
IMAGE_REPO="${IMAGE%:*}"
IMAGE_TAG="${IMAGE##*:}"
PORT_FORWARD_PORT="${KNECT_HELM_PORT_FORWARD_PORT:-18420}"
PF_PID=""

# lib.sh already pinned KUBECONFIG to /tmp/knect-e2e-kubeconfig and refuses to
# inherit the caller's environment.
if [[ ! -s "${KUBECONFIG}" ]]; then
	if cluster_exists; then
		log "exporting kubeconfig for existing Kind cluster '${KIND_CLUSTER_NAME}'"
		kind export kubeconfig --name "${KIND_CLUSTER_NAME}" --kubeconfig "${KUBECONFIG}"
	else
		die "Kind cluster '${KIND_CLUSTER_NAME}' does not exist; run hack/setup-kind-e2e.sh first"
	fi
fi
assert_e2e_context

cleanup() {
	local rc=$?
	trap - EXIT
	if [[ -n "${PF_PID}" ]]; then
		kill "${PF_PID}" >/dev/null 2>&1 || true
		wait "${PF_PID}" >/dev/null 2>&1 || true
	fi
	if helm status "${RELEASE}" --namespace "${NAMESPACE}" >/dev/null 2>&1; then
		log "uninstalling Helm release ${RELEASE} from ${NAMESPACE}"
		helm uninstall "${RELEASE}" --namespace "${NAMESPACE}" >/dev/null 2>&1 || true
	fi
	kubectl_e2e delete namespace "${NAMESPACE}" --wait=false >/dev/null 2>&1 || true
	if [[ "${rc}" -eq 0 ]]; then
		ok "kind Helm smoke test complete; cluster left running"
	else
		err "kind Helm smoke test failed (rc=${rc}); cluster left running for debugging"
	fi
	exit "${rc}"
}
trap cleanup EXIT

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
	log "building ${IMAGE} for ${PLATFORM}"
	docker build --platform "${PLATFORM}" --tag "${IMAGE}" \
		--build-arg VERSION=helm-local "${KNECT_REPO_ROOT}"
else
	docker image inspect "${IMAGE}" >/dev/null 2>&1 || die "image not found: ${IMAGE}"
fi

log "linting chart"
helm lint "${KNECT_REPO_ROOT}/charts/knectd"

log "rendering chart"
helm template "${RELEASE}" "${KNECT_REPO_ROOT}/charts/knectd" \
	--namespace "${NAMESPACE}" >/tmp/knectd-helm-test-rendered.yaml
test -s /tmp/knectd-helm-test-rendered.yaml

log "creating namespace ${NAMESPACE}"
kubectl_e2e create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl_e2e apply -f -

log "server-side dry-run of rendered manifests"
kubectl_e2e apply --dry-run=server -f /tmp/knectd-helm-test-rendered.yaml >/dev/null

log "loading ${IMAGE} into Kind cluster ${KIND_CLUSTER_NAME}"
kind load docker-image --name "${KIND_CLUSTER_NAME}" "${IMAGE}"

log "installing ${RELEASE} in ${NAMESPACE}"
helm upgrade --install "${RELEASE}" "${KNECT_REPO_ROOT}/charts/knectd" \
	--namespace "${NAMESPACE}" \
	--set image.repository="${IMAGE_REPO}" \
	--set image.tag="${IMAGE_TAG}" \
	--set image.pullPolicy=Never \
	--wait \
	--timeout 5m \
	--atomic

log "waiting for the deployment"
kubectl_e2e -n "${NAMESPACE}" wait --for=condition=Available \
	deployment -l "app.kubernetes.io/instance=${RELEASE}" --timeout=180s

log "installed resources"
kubectl_e2e -n "${NAMESPACE}" get deploy,pod,svc,sa,pdb
kubectl_e2e get clusterrole,clusterrolebinding | grep -F "${RELEASE}" || true

log "service account permissions"
kubectl_e2e -n "${NAMESPACE}" auth can-i --list \
	--as="system:serviceaccount:${NAMESPACE}:$(kubectl_e2e -n "${NAMESPACE}" get sa -l "app.kubernetes.io/instance=${RELEASE}" -o jsonpath='{.items[0].metadata.name}')"

if [[ "${SKIP_PORT_FORWARD}" -eq 0 ]]; then
	svc="$(kubectl_e2e -n "${NAMESPACE}" get svc -l "app.kubernetes.io/instance=${RELEASE}" -o jsonpath='{.items[0].metadata.name}')"
	log "forwarding ${svc}:7420 to 127.0.0.1:${PORT_FORWARD_PORT}"
	kubectl_e2e -n "${NAMESPACE}" port-forward "service/${svc}" \
		"${PORT_FORWARD_PORT}:7420" >/tmp/knectd-helm-port-forward.log 2>&1 &
	PF_PID=$!

	ready=0
	for _ in $(seq 1 60); do
		if curl -fsS "http://127.0.0.1:${PORT_FORWARD_PORT}/healthz" >/tmp/knectd-helm-healthz.json 2>/dev/null; then
			ready=1
			break
		fi
		sleep 1
	done
	[[ "${ready}" -eq 1 ]] || die "port-forward health check did not become ready"
	ok "healthz: $(cat /tmp/knectd-helm-healthz.json)"
	curl -fsS "http://127.0.0.1:${PORT_FORWARD_PORT}/" >/tmp/knectd-helm-index.html
	grep -q '<title>knect' /tmp/knectd-helm-index.html || die "embedded WebUI index was not served"
	ok "embedded WebUI served at /"
fi

ok "kind Helm smoke test passed"
