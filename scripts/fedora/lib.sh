#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"

info() {
	printf '[kait2en] %s\n' "$*"
}

fail() {
	printf '[kait2en] error: %s\n' "$*" >&2
	exit 1
}

require_root() {
	[[ ${EUID:-$(id -u)} -eq 0 ]] || fail "run this script with sudo"
}

require_repo_root() {
	[[ -d "$REPO_ROOT/modules" && -d "$REPO_ROOT/apps" ]] ||
		fail "repository layout is incomplete"
}

require_fedora() {
	[[ -r /etc/os-release ]] || fail "/etc/os-release is missing"
	# shellcheck disable=SC1091
	. /etc/os-release
	[[ ${ID:-} == fedora || " ${ID_LIKE:-} " == *" fedora "* ]] ||
		fail "this script is Fedora-only"
}

require_command() {
	local cmd
	for cmd in "$@"; do
		command -v "$cmd" >/dev/null 2>&1 || fail "missing command: $cmd"
	done
}

kernel_release() {
	printf '%s\n' "${KERNEL_RELEASE:-$(uname -r)}"
}

require_min_kernel() {
	local min_major=$1 min_minor=$2 release major minor

	release="$(kernel_release)"
	if [[ ! "$release" =~ ^([0-9]+)\.([0-9]+) ]]; then
		fail "unable to determine Linux kernel version from: $release"
	fi

	major="${BASH_REMATCH[1]}"
	minor="${BASH_REMATCH[2]}"

	if (( major < min_major || (major == min_major && minor < min_minor) )); then
		fail "KaiT2en requires Linux kernel ${min_major}.${min_minor} or newer. Update Fedora first, reboot into the updated kernel, then run this installer again. Current kernel: $release"
	fi
}
