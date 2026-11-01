#!/usr/bin/env bash
# Render the krew plugin manifest from a release checksums file.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: render-krew.sh --tag TAG --repository OWNER/REPO --checksums FILE \
         --output FILE [--template FILE]

TAG is the git tag (for example v1.0.0). The rendered manifest is what
krew-release-bot submits to krew-index.
EOF
}

tag=""
repository=""
checksums=""
output=""
template="install/krew/knect.yaml.tpl"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag)
      tag="${2:-}"
      shift 2
      ;;
    --repository)
      repository="${2:-}"
      shift 2
      ;;
    --checksums)
      checksums="${2:-}"
      shift 2
      ;;
    --output)
      output="${2:-}"
      shift 2
      ;;
    --template)
      template="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "${tag}" ] || [ -z "${repository}" ] || [ -z "${checksums}" ] || [ -z "${output}" ]; then
  echo "missing required argument" >&2
  usage >&2
  exit 2
fi

if ! [[ "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$ ]]; then
  echo "invalid tag: ${tag}" >&2
  exit 2
fi
if ! [[ "${repository}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "invalid repository: ${repository}" >&2
  exit 2
fi
if [ ! -f "${checksums}" ]; then
  echo "checksums file not found: ${checksums}" >&2
  exit 2
fi
if [ ! -f "${template}" ]; then
  echo "template not found: ${template}" >&2
  exit 2
fi

sha_for() {
  local name="$1"
  awk -v name="${name}" '$2 == name { print $1; found = 1 } END { if (!found) exit 1 }' "${checksums}"
}

linux_amd64="$(sha_for "knect_${tag}_linux_amd64.tar.gz")"
linux_arm64="$(sha_for "knect_${tag}_linux_arm64.tar.gz")"
darwin_amd64="$(sha_for "knect_${tag}_darwin_amd64.tar.gz")"
darwin_arm64="$(sha_for "knect_${tag}_darwin_arm64.tar.gz")"

mkdir -p "$(dirname "${output}")"
sed \
  -e "s|@@REPOSITORY@@|${repository}|g" \
  -e "s|@@TAG@@|${tag}|g" \
  -e "s|@@SHA256_LINUX_AMD64@@|${linux_amd64}|g" \
  -e "s|@@SHA256_LINUX_ARM64@@|${linux_arm64}|g" \
  -e "s|@@SHA256_DARWIN_AMD64@@|${darwin_amd64}|g" \
  -e "s|@@SHA256_DARWIN_ARM64@@|${darwin_arm64}|g" \
  "${template}" > "${output}"

if grep -q '@@[A-Z0-9_]*@@' "${output}"; then
  echo "rendered manifest still contains template placeholders: ${output}" >&2
  exit 1
fi

if command -v yq >/dev/null 2>&1 && yq --version 2>&1 | grep -q mikefarah; then
  yq eval '.' "${output}" >/dev/null
elif python3 -c 'import yaml' >/dev/null 2>&1; then
  python3 - "${output}" <<'PY'
import sys
import yaml

with open(sys.argv[1], encoding="utf-8") as handle:
    yaml.safe_load(handle)
PY
fi

echo "rendered ${output}"
