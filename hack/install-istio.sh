#!/usr/bin/env bash
# Install Istio 1.29.2 with the minimal profile plus the ingress gateway that
# Knative net-istio and Flagger's Istio provider both need. Idempotent.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ISTIO_VERSION="${ISTIO_VERSION:-1.29.2}"
ISTIO_NAMESPACE="${ISTIO_NAMESPACE:-istio-system}"

require_cmds kubectl curl tar
assert_e2e_context

istioctl_bin="${ISTIOCTL:-}"
if [[ -z "${istioctl_bin}" ]]; then
	istioctl_bin="$(install_istioctl "${ISTIO_VERSION}")"
fi
[[ -x "${istioctl_bin}" ]] || die "istioctl not executable: ${istioctl_bin}"

log "installing Istio ${ISTIO_VERSION} (profile=minimal + ingressgateway)"
"${istioctl_bin}" install \
	--kubeconfig "${KUBECONFIG}" \
	--context "${KIND_CONTEXT}" \
	-y \
	--set profile=minimal \
	--set components.ingressGateways[0].enabled=true \
	--set components.ingressGateways[0].name=istio-ingressgateway \
	--set components.ingressGateways[0].k8s.service.type=NodePort \
	--set values.global.proxy.resources.requests.cpu=10m \
	--set values.global.proxy.resources.requests.memory=64Mi

relax_cpu_requests "${ISTIO_NAMESPACE}"
wait_for_deployment "${ISTIO_NAMESPACE}" istiod 600s
wait_for_deployment "${ISTIO_NAMESPACE}" istio-ingressgateway 600s

# Kind maps host 80/443/7420 to node ports 30080/30443/30020 (see
# kind-config.yaml). Pin the gateway Service to those node ports.
log "pinning istio-ingressgateway to node ports 30080/30443"
kubectl_e2e -n "${ISTIO_NAMESPACE}" patch service istio-ingressgateway --type=strategic \
	-p '{"spec":{"type":"NodePort","ports":[{"port":80,"nodePort":30080},{"port":443,"nodePort":30443}]}}' >/dev/null

# Only the fixture workload namespace is opted into sidecar injection.
# Labelling knative-serving/knative-eventing would inject sidecars into the
# control planes and is not needed by net-istio.
kubectl_e2e get namespace "${KNECT_E2E_NAMESPACE}" >/dev/null 2>&1 ||
	kubectl_e2e create namespace "${KNECT_E2E_NAMESPACE}"
kubectl_e2e label namespace "${KNECT_E2E_NAMESPACE}" istio-injection=enabled --overwrite >/dev/null || true

ok "Istio ${ISTIO_VERSION} ready"
