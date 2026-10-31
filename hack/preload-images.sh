#!/usr/bin/env bash
# Pull every pinned image in hack/images.txt in parallel and load it into the
# knect-e2e Kind node. Missing images are reported but do not fail the run
# unless KNECT_PRELOAD_STRICT=1 (or --strict) is set.
#
# Usage:
#   hack/preload-images.sh [--images FILE] [--pull-only] [--strict]
#
# Exit codes: 0 completed (possibly with skipped images), 1 strict failure,
#             2 wrong Kubernetes context / no cluster.

set -Eeuo pipefail

# shellcheck source=lib.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

IMAGES_FILE="${KNECT_IMAGES_FILE:-${KNECT_HACK_DIR}/images.txt}"
PULL_ONLY=0
STRICT="${KNECT_PRELOAD_STRICT:-0}"
JOBS="${KNECT_PULL_JOBS:-4}"

usage() {
	sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--images)
		IMAGES_FILE="$2"
		shift 2
		;;
	--pull-only)
		PULL_ONLY=1
		shift
		;;
	--strict)
		STRICT=1
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

require_cmds docker kind
[[ -r "${IMAGES_FILE}" ]] || die "images file not readable: ${IMAGES_FILE}"
ensure_cache_dir

if [[ "${PULL_ONLY}" -eq 0 ]]; then
	cluster_exists || die "Kind cluster '${KIND_CLUSTER_NAME}' does not exist; run setup-kind-e2e.sh first"
	assert_e2e_context
fi

mapfile -t images < <(sed -e 's/#.*$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "${IMAGES_FILE}" | grep -v '^$')
[[ "${#images[@]}" -gt 0 ]] || die "no images found in ${IMAGES_FILE}"

log "preloading ${#images[@]} images with ${JOBS} parallel pulls"

save_cache=0
if [[ "${KNECT_E2E_SAVE_IMAGE_CACHE:-0}" == "1" ]]; then
	save_cache=1
fi
cache_key="$(sha256sum "${IMAGES_FILE}" | cut -c1-16)"
cache_tar="${KNECT_E2E_CACHE_DIR}/images-${cache_key}.tar"
if [[ -f "${cache_tar}" ]]; then
	log "loading Docker image cache ${cache_tar}"
	docker load -i "${cache_tar}" || warn "cache load failed; continuing with registry pulls"
fi

log_dir="${KNECT_E2E_CACHE_DIR}/pull-logs"
rm -rf "${log_dir}"
mkdir -p "${log_dir}"

pull_one() {
	local image="$1"
	local log_file="${log_dir}/$(printf '%s' "${image}" | tr '/:@' '___').log"
	if docker image inspect "${image}" >/dev/null 2>&1; then
		printf 'present %s\n' "${image}"
		return 0
	fi
	if docker pull "${image}" >"${log_file}" 2>&1; then
		printf 'pulled %s\n' "${image}"
		return 0
	fi
	printf 'failed %s\n' "${image}"
	return 1
}
export -f pull_one
export log_dir

set +e
printf '%s\n' "${images[@]}" |
	xargs -r -n1 -P "${JOBS}" bash -c 'pull_one "$1"' _ >"${KNECT_E2E_CACHE_DIR}/pull-results.txt" 2>&1
xargs_rc=$?
set -e
if [[ "${xargs_rc}" -ne 0 ]]; then
	warn "some image pulls failed (xargs rc=${xargs_rc}); see ${log_dir}"
fi

pulled=()
failed=()
while read -r state image; do
	case "${state}" in
	present | pulled) pulled+=("${image}") ;;
	failed) failed+=("${image}") ;;
	esac
done <"${KNECT_E2E_CACHE_DIR}/pull-results.txt"

ok "image pulls: ${#pulled[@]} available, ${#failed[@]} failed"
if [[ "${#failed[@]}" -gt 0 ]]; then
	for image in "${failed[@]}"; do
		log_file="${log_dir}/$(printf '%s' "${image}" | tr '/:@' '___').log"
		warn "missing image: ${image}"
		if [[ -s "${log_file}" ]]; then
			tail -n 3 "${log_file}" | sed 's/^/    /' >&2 || true
		fi
	done
fi

if [[ "${save_cache}" -eq 1 && "${#pulled[@]}" -gt 0 && ! -f "${cache_tar}" ]]; then
	log "saving Docker image cache to ${cache_tar}"
	if docker save -o "${cache_tar}.tmp" "${pulled[@]}"; then
		mv "${cache_tar}.tmp" "${cache_tar}"
		ok "saved image cache ($(du -sh "${cache_tar}" | cut -f1))"
	else
		rm -f "${cache_tar}.tmp"
		warn "failed to save Docker image cache"
	fi
fi

if [[ "${PULL_ONLY}" -eq 1 ]]; then
	[[ "${STRICT}" -eq 0 || "${#failed[@]}" -eq 0 ]] || exit 1
	exit 0
fi

if ! cluster_exists; then
	warn "cluster disappeared before image load; skipping kind load"
	[[ "${STRICT}" -eq 0 ]] || exit 1
	exit 0
fi

load_failed=()
for image in "${pulled[@]}"; do
	if kind_e2e load docker-image --name "${KIND_CLUSTER_NAME}" "${image}" >/dev/null 2>&1; then
		log "loaded ${image}"
	else
		warn "kind load failed: ${image}"
		load_failed+=("${image}")
	fi
done

if [[ "${#failed[@]}" -gt 0 || "${#load_failed[@]}" -gt 0 ]]; then
	warn "preload finished with issues: ${#failed[@]} missing pull(s), ${#load_failed[@]} failed load(s)"
	[[ "${STRICT}" -eq 0 ]] || exit 1
else
	ok "all ${#pulled[@]} images loaded into ${KIND_CLUSTER_NAME}"
fi
