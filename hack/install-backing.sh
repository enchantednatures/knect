#!/usr/bin/env bash
# Install the backing services and optional operator CRDs used by the fixture
# matrix: Redis, CloudNativePG Postgres, Redpanda, RabbitMQ, mock Ceph/Rook
# CRDs, plus CRDs-only Strimzi/Redpanda Topic/RedisCluster/Dragonfly objects.
# Idempotent.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

CNPG_VERSION="${CNPG_VERSION:-1.30.1}"
ROOK_VERSION="${ROOK_VERSION:-v1.21.1}"
DRAGONFLY_OPERATOR_VERSION="${DRAGONFLY_OPERATOR_VERSION:-v1.7.1}"
REDIS_OPERATOR_VERSION="${REDIS_OPERATOR_VERSION:-v0.27.0}"
REDPANDA_OPERATOR_TAG="${REDPANDA_OPERATOR_TAG:-operator/v26.2.5}"

require_cmds kubectl curl
assert_e2e_context

kubectl_e2e get namespace "${KNECT_E2E_NAMESPACE}" >/dev/null 2>&1 ||
	kubectl_e2e create namespace "${KNECT_E2E_NAMESPACE}"

log "installing CloudNativePG ${CNPG_VERSION}"
apply_url "https://github.com/cloudnative-pg/cloudnative-pg/releases/download/v${CNPG_VERSION}/cnpg-${CNPG_VERSION}.yaml"
wait_for_crd 300s clusters.postgresql.cnpg.io
wait_for_deployment cnpg-system cnpg-controller-manager 600s

log "installing Rook ${ROOK_VERSION} CRDs only (mock Ceph storage)"
apply_url "https://raw.githubusercontent.com/rook/rook/${ROOK_VERSION}/deploy/examples/crds.yaml"
wait_for_crd 300s \
	cephblockpools.ceph.rook.io \
	cephfilesystems.ceph.rook.io

log "installing optional operator CRDs (no controllers: fixture-only shapes)"
apply_url "https://raw.githubusercontent.com/dragonflydb/dragonfly-operator/${DRAGONFLY_OPERATOR_VERSION}/config/crd/bases/dragonflydb.io_dragonflies.yaml"
apply_url "https://raw.githubusercontent.com/OT-container-kit/redis-operator/${REDIS_OPERATOR_VERSION}/config/crd/bases/redis.redis.opstreelabs.in_redisclusters.yaml"
apply_url "https://raw.githubusercontent.com/redpanda-data/redpanda-operator/${REDPANDA_OPERATOR_TAG}/operator/config/crd/bases/cluster.redpanda.com_topics.yaml"
wait_for_crd 300s \
	dragonflies.dragonflydb.io \
	redisclusters.redis.redis.opstreelabs.in \
	topics.cluster.redpanda.com

log "applying core backing-service fixtures"
apply_dir "${KNECT_HACK_DIR}/fixtures/infra"

wait_ready --namespace "${KNECT_E2E_NAMESPACE}" --timeout 900
ok "backing services ready in namespace ${KNECT_E2E_NAMESPACE}"
