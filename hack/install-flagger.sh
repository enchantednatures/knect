#!/usr/bin/env bash
# Install Flagger 1.45.0 with the Istio mesh provider, no Prometheus, and a
# short 10s control loop suitable for E2E tests. Idempotent.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

FLAGGER_CHART_VERSION="${FLAGGER_CHART_VERSION:-1.45.0}"
FLAGGER_NAMESPACE="${FLAGGER_NAMESPACE:-flagger}"

require_cmds helm kubectl
assert_e2e_context

helm repo add flagger https://flagger.app >/dev/null 2>&1 || true
helm repo update flagger >/dev/null

log "installing Flagger ${FLAGGER_CHART_VERSION} (meshProvider=istio, interval=10s)"
helm upgrade --install flagger flagger/flagger \
	--namespace "${FLAGGER_NAMESPACE}" \
	--create-namespace \
	--version "${FLAGGER_CHART_VERSION}" \
	--set image.tag="${FLAGGER_CHART_VERSION}" \
	--set meshProvider=istio \
	--set metricsServer=http://flagger-noop.invalid:9090 \
	--set 'env[0].name=CONTROL_LOOP_INTERVAL' \
	--set 'env[0].value=10s' \
	--wait=false
relax_cpu_requests "${FLAGGER_NAMESPACE}"
wait_for_deployment "${FLAGGER_NAMESPACE}" flagger 5m

kubectl_e2e wait --for=condition=Established --timeout=300s \
	crd/canaries.flagger.app \
	crd/metrictemplates.flagger.app

ok "Flagger ${FLAGGER_CHART_VERSION} ready"
