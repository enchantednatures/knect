#!/usr/bin/env bash
# Install Kyverno 1.19.2 (chart 3.9.2) with one replica per active controller
# and reports enabled. Idempotent.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

KYVERNO_CHART_VERSION="${KYVERNO_CHART_VERSION:-3.9.2}"
KYVERNO_APP_VERSION="${KYVERNO_APP_VERSION:-v1.19.2}"
KYVERNO_NAMESPACE="${KYVERNO_NAMESPACE:-kyverno}"

require_cmds helm kubectl
assert_e2e_context

helm repo add kyverno https://kyverno.github.io/kyverno/ >/dev/null 2>&1 || true
helm repo update kyverno >/dev/null

log "installing Kyverno ${KYVERNO_APP_VERSION} (chart ${KYVERNO_CHART_VERSION})"
helm upgrade --install kyverno kyverno/kyverno \
	--namespace "${KYVERNO_NAMESPACE}" \
	--create-namespace \
	--version "${KYVERNO_CHART_VERSION}" \
	--set admissionController.replicas=1 \
	--set backgroundController.replicas=1 \
	--set reportsController.replicas=1 \
	--set cleanupController.enabled=false \
	--set test.image.tag="${KYVERNO_APP_VERSION}" \
	--set webhooksCleanup.image.tag="${KYVERNO_APP_VERSION}" \
	--wait=false
relax_cpu_requests "${KYVERNO_NAMESPACE}"
wait_ready --namespace "${KYVERNO_NAMESPACE}" --timeout 600

kubectl_e2e wait --for=condition=Established --timeout=300s \
	crd/clusterpolicies.kyverno.io \
	crd/policies.kyverno.io \
	crd/policyreports.wgpolicyk8s.io \
	crd/clusterpolicyreports.wgpolicyk8s.io

ok "Kyverno ${KYVERNO_APP_VERSION} ready"
