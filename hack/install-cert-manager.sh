#!/usr/bin/env bash
# Install cert-manager v1.21.3 (required by the RabbitMQ Messaging Topology
# Operator webhooks). Idempotent.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

CERT_MANAGER_VERSION="${CERT_MANAGER_VERSION:-v1.21.3}"
CERT_MANAGER_MANIFEST="${CERT_MANAGER_MANIFEST:-https://github.com/cert-manager/cert-manager/releases/download/${CERT_MANAGER_VERSION}/cert-manager.yaml}"

require_cmds kubectl curl
assert_e2e_context

log "installing cert-manager ${CERT_MANAGER_VERSION}"
apply_url "${CERT_MANAGER_MANIFEST}"

wait_for_crd 300s \
	certificates.cert-manager.io \
	issuers.cert-manager.io \
	clusterissuers.cert-manager.io

for deployment in cert-manager cert-manager-cainjector cert-manager-webhook; do
	wait_for_deployment cert-manager "${deployment}" 300s
done

# Wait for the webhook TLS secret/endpoints before the topology operator starts.
kubectl_e2e -n cert-manager rollout status deployment/cert-manager-webhook --timeout=300s
ok "cert-manager ready"
