#!/usr/bin/env bash
# Delete ONLY the disposable knect-e2e Kind cluster and its isolated
# kubeconfig. Safe to run when nothing exists.
#
# Never deletes the user's default kubeconfig, any other KinD cluster, or any
# cluster referenced by ~/.kube/config.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

if [[ "${KIND_CLUSTER_NAME}" != "knect-e2e" || "${KIND_CONTEXT}" != "kind-knect-e2e" ]]; then
	die "refusing teardown: expected cluster knect-e2e/context kind-knect-e2e, got ${KIND_CLUSTER_NAME}/${KIND_CONTEXT}"
fi

require_cmds kind

started="${SECONDS}"
if kind get clusters 2>/dev/null | grep -Fxq "${KIND_CLUSTER_NAME}"; then
	log "deleting Kind cluster ${KIND_CLUSTER_NAME}"
	kind delete cluster --name "${KIND_CLUSTER_NAME}" --kubeconfig "${KUBECONFIG}"
	ok "deleted Kind cluster ${KIND_CLUSTER_NAME} ($((SECONDS - started))s)"
else
	log "Kind cluster ${KIND_CLUSTER_NAME} does not exist; nothing to delete"
fi

if [[ "${KUBECONFIG}" == "/tmp/knect-e2e-kubeconfig" ]]; then
	if [[ -e "${KUBECONFIG}" ]]; then
		rm -f "${KUBECONFIG}"
		ok "removed isolated kubeconfig ${KUBECONFIG}"
	else
		log "isolated kubeconfig ${KUBECONFIG} already absent"
	fi
else
	warn "KUBECONFIG is ${KUBECONFIG}; refusing to remove a non-standard kubeconfig path"
fi
