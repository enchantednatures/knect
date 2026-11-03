#!/usr/bin/env bash
# Install Knative Serving, Eventing, Kafka Broker and RabbitMQ Broker at 1.24.0
# (net-istio 1.24 is the pinned Istio 1.29-compatible networking layer).
# Idempotent; every remote manifest is pinned to a release tag.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

KNATIVE_VERSION="${KNATIVE_VERSION:-1.24.0}"
KNATIVE_TAG="knative-v${KNATIVE_VERSION}"
SERVING_BASE="https://github.com/knative/serving/releases/download/${KNATIVE_TAG}"
NET_ISTIO_BASE="https://github.com/knative/net-istio/releases/download/${KNATIVE_TAG}"
EVENTING_BASE="https://github.com/knative/eventing/releases/download/${KNATIVE_TAG}"
KAFKA_BASE="https://github.com/knative-extensions/eventing-kafka-broker/releases/download/${KNATIVE_TAG}"
RABBITMQ_BASE="https://github.com/knative-extensions/eventing-rabbitmq/releases/download/${KNATIVE_TAG}"
RABBITMQ_CLUSTER_OPERATOR_VERSION="${RABBITMQ_CLUSTER_OPERATOR_VERSION:-v2.23.2}"
RABBITMQ_TOPOLOGY_OPERATOR_VERSION="${RABBITMQ_TOPOLOGY_OPERATOR_VERSION:-v1.20.5}"
STRIMZI_VERSION="${STRIMZI_VERSION:-1.3.0}"

require_cmds kubectl curl
assert_e2e_context

install_serving() {
	log "installing Knative Serving ${KNATIVE_VERSION}"
	apply_url_client "${SERVING_BASE}/serving-crds.yaml"
	wait_for_crd 300s \
		services.serving.knative.dev \
		configurations.serving.knative.dev \
		revisions.serving.knative.dev \
		routes.serving.knative.dev

	apply_url_client "${SERVING_BASE}/serving-core.yaml"
	apply_url_client "${NET_ISTIO_BASE}/net-istio.yaml"
	relax_cpu_requests knative-serving

	# net-istio is the only networking layer; make it the cluster ingress class.
	kubectl_e2e -n knative-serving patch configmap config-network --type merge \
		-p '{"data":{"ingress.class":"istio.ingress.networking.knative.dev"}}' >/dev/null
	# Use sslip.io directly. The default-domain magic-DNS job waits for a
	# LoadBalancer ingress IP, which a Kind NodePort gateway never receives.
	kubectl_e2e -n knative-serving patch configmap config-domain --type merge \
		-p '{"data":{"sslip.io":""}}' >/dev/null
	kubectl_e2e -n knative-serving patch configmap config-features --type merge \
		-p '{"data":{"kubernetes.podspec-persistent-volume-claim":"enabled","kubernetes.podspec-persistent-volume-write":"enabled"}}' >/dev/null
	kubectl_e2e -n knative-serving patch configmap config-deployment --type merge \
		-p '{"data":{"queue-sidecar-cpu-request":"10m","queue-sidecar-memory-request":"64Mi"}}' >/dev/null

	wait_ready --namespace knative-serving --timeout 600
	ok "Knative Serving ${KNATIVE_VERSION} ready"
}

install_eventing() {
	log "installing Knative Eventing ${KNATIVE_VERSION}"
	apply_url_client "${EVENTING_BASE}/eventing-crds.yaml"
	wait_for_crd 300s \
		brokers.eventing.knative.dev \
		triggers.eventing.knative.dev \
		channels.messaging.knative.dev \
		subscriptions.messaging.knative.dev

	apply_url_client "${EVENTING_BASE}/eventing-core.yaml"
	apply_url_client "${EVENTING_BASE}/in-memory-channel.yaml"
	apply_url_client "${EVENTING_BASE}/mt-channel-broker.yaml"
	relax_cpu_requests knative-eventing

	wait_ready --namespace knative-eventing --timeout 600
	ok "Knative Eventing ${KNATIVE_VERSION} ready"
}

install_kafka() {
	log "installing Knative Kafka Broker ${KNATIVE_VERSION} + Strimzi CRDs ${STRIMZI_VERSION}"
	apply_url_client "https://github.com/strimzi/strimzi-kafka-operator/releases/download/${STRIMZI_VERSION}/strimzi-crds-${STRIMZI_VERSION}.yaml"
	apply_url_client "${KAFKA_BASE}/eventing-kafka-controller.yaml"
	apply_url_client "${KAFKA_BASE}/eventing-kafka-broker.yaml"
	apply_url_client "${KAFKA_BASE}/eventing-kafka-source.yaml"
	relax_cpu_requests knative-eventing

	kubectl_e2e -n knative-eventing create configmap kafka-broker-config \
		--from-literal=bootstrap.servers=redpanda.${KNECT_E2E_NAMESPACE}.svc.cluster.local:9092 \
		--from-literal=default.topic.replication.factor=1 \
		--from-literal=default.topic.partitions=1 \
		--dry-run=client -o yaml |
		kubectl_e2e apply -f - >/dev/null

	kubectl_e2e wait --for=condition=Established --timeout=300s \
		crd/kafkas.kafka.strimzi.io \
		crd/kafkatopics.kafka.strimzi.io \
		crd/kafkasources.sources.knative.dev
	ok "Knative Kafka Broker ${KNATIVE_VERSION} manifests applied"
}

install_rabbitmq() {
	log "installing RabbitMQ Cluster Operator ${RABBITMQ_CLUSTER_OPERATOR_VERSION} + Messaging Topology Operator ${RABBITMQ_TOPOLOGY_OPERATOR_VERSION}"
	apply_url_client "https://github.com/rabbitmq/cluster-operator/releases/download/${RABBITMQ_CLUSTER_OPERATOR_VERSION}/cluster-operator.yml"
	apply_url_client "https://github.com/rabbitmq/messaging-topology-operator/releases/download/${RABBITMQ_TOPOLOGY_OPERATOR_VERSION}/messaging-topology-operator-with-certmanager.yaml"
	apply_url_client "${RABBITMQ_BASE}/rabbitmq-broker.yaml"
	apply_url_client "${RABBITMQ_BASE}/rabbitmq-source.yaml"
	relax_cpu_requests knative-eventing
	relax_cpu_requests knative-sources

	wait_for_crd 300s \
		rabbitmqclusters.rabbitmq.com \
		rabbitmqbrokerconfigs.eventing.knative.dev
	wait_for_deployment rabbitmq-system rabbitmq-cluster-operator 600s
	wait_for_deployment rabbitmq-system messaging-topology-operator 600s
	ok "RabbitMQ broker components ready"
}

install_serving
install_eventing
install_kafka
install_rabbitmq

relax_cpu_requests knative-serving
relax_cpu_requests knative-eventing
relax_cpu_requests knative-sources

log "waiting for all Knative control planes"
wait_ready --namespace knative-eventing --timeout 900
wait_ready --namespace knative-serving --timeout 600
ok "Knative stack ${KNATIVE_VERSION} ready"
