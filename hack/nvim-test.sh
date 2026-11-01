#!/usr/bin/env bash
# Run the knect.nvim Plenary suite in headless Neovim.
#
# Test-only dependencies (Plenary, Nui, Telescope) are cloned into a cache
# directory and reused across runs. The plugin itself has no runtime Lua
# dependencies.

set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
NVIM_DIR="${REPO_ROOT}/nvim"
DEPS_DIR="${KNECT_NVIM_DEPS_DIR:-${XDG_CACHE_HOME:-${HOME}/.cache}/knect-nvim-deps}"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/knect-nvim-test.XXXXXX")"

log() {
	printf '[nvim-test] %s\n' "$*"
}

cleanup() {
	rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

if ! command -v nvim >/dev/null 2>&1; then
	log "error: nvim is not installed"
	exit 127
fi
if ! command -v git >/dev/null 2>&1; then
	log "error: git is required to fetch test-only plugins"
	exit 127
fi

log "$(nvim --version | head -n 1)"
log "repository: ${REPO_ROOT}"
log "dependencies: ${DEPS_DIR}"
log "scratch: ${WORK_DIR}"

mkdir -p "${DEPS_DIR}"

clone_dep() {
	local name="$1"
	local url="$2"
	local dest="${DEPS_DIR}/${name}"

	if [[ -d "${dest}/.git" || -f "${dest}/.knect-ready" ]]; then
		log "${name}: using cached checkout"
		return 0
	fi

	rm -rf "${dest}"
	log "${name}: cloning ${url}"
	if ! GIT_TERMINAL_PROMPT=0 git clone --depth 1 --single-branch "${url}" "${dest}"; then
		rm -rf "${dest}"
		log "${name}: clone failed (network access is required on first run)"
		return 1
	fi
	touch "${dest}/.knect-ready"
}

clone_dep plenary.nvim https://github.com/nvim-lua/plenary.nvim.git
clone_dep nui.nvim https://github.com/MunifTanjim/nui.nvim.git
clone_dep telescope.nvim https://github.com/nvim-telescope/telescope.nvim.git

export KNECT_REPO_ROOT="${REPO_ROOT}"
export KNECT_NVIM_DEPS="${DEPS_DIR}"
export KNECT_TEST_TMP="${WORK_DIR}"
export TMPDIR="${WORK_DIR}"

cd "${NVIM_DIR}"

log "running PlenaryBustedDirectory tests/knect"
set +e
nvim --headless -u tests/minimal_init.lua -i NONE \
	-c "PlenaryBustedDirectory tests/knect { minimal_init = 'tests/minimal_init.lua' }"
status=$?
set -e

if [[ ${status} -eq 0 ]]; then
	log "suite passed"
else
	log "suite failed with exit code ${status}"
fi
exit "${status}"
