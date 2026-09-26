#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"

# One error ledger for the complete installation, inherited by child scripts.
# A failing atomic step stops only that step; run_step continues its siblings.
INSTALL_REPORT_OWNER=0
if [[ -z "${KAIT2EN_INSTALL_ERRORS:-}" ]]; then
	KAIT2EN_INSTALL_ERRORS=$(mktemp /tmp/kait2en-install-errors.XXXXXX)
	export KAIT2EN_INSTALL_ERRORS
	INSTALL_REPORT_OWNER=1
fi

KAIT2EN_INSTALL_HARD_ERRORS="$KAIT2EN_INSTALL_ERRORS.hard"

record_error() {
	local message="$*"
	printf '[kait2en] error: %s\n' "$message" >&2
	if ! printf '%s: %s\n' "${0##*/}" "$message" >>"$KAIT2EN_INSTALL_ERRORS"; then
		printf '[kait2en] error: cannot append to error report %s\n' "$KAIT2EN_INSTALL_ERRORS" >&2
	fi
	printf '%s: %s\n' "${0##*/}" "$message" >>"$KAIT2EN_INSTALL_HARD_ERRORS" || true
}

installation_summary() {
	# The installer runs under sudo, so the report was root-owned throughout
	# the run. Hand it to the invoking user now that nothing else writes to it.
	if [[ ${EUID:-$(id -u)} -eq 0 && -n "${SUDO_USER:-}" ]]; then
		chown "$SUDO_USER" "$KAIT2EN_INSTALL_ERRORS" "$KAIT2EN_INSTALL_HARD_ERRORS" 2>/dev/null || true
	fi
	if [[ -s "$KAIT2EN_INSTALL_HARD_ERRORS" ]]; then
		printf '[kait2en] installation completed with errors:\n' >&2
		cat "$KAIT2EN_INSTALL_ERRORS" >&2
		printf '[kait2en] saved report: %s\n' "$KAIT2EN_INSTALL_ERRORS" >&2
		return 1
	fi
	if [[ -s "$KAIT2EN_INSTALL_ERRORS" ]]; then
		printf '[kait2en] installation completed with warnings:\n' >&2
		cat "$KAIT2EN_INSTALL_ERRORS" >&2
		printf '[kait2en] saved report: %s\n' "$KAIT2EN_INSTALL_ERRORS" >&2
		return 0
	fi
	info "installation completed without recorded errors"
}

installer_exit() {
	local status=$1
	trap - ERR EXIT
	if (( INSTALL_REPORT_OWNER )); then
		installation_summary || status=1
	fi
	exit "$status"
}

trap 'record_error "command failed (status $?) at ${BASH_SOURCE[0]:-$0}:$LINENO: $BASH_COMMAND"' ERR
trap 'installer_exit "$?"' EXIT

run_step() {
	local label=$1
	shift
	info "$label"
	# Do not put the subshell in an if/! expression: Bash would then disable
	# errexit inside shell functions and execute unsafe dependent commands.
	set +e
	(
		set -Eeuo pipefail
		INSTALL_REPORT_OWNER=0
		# Parent cleanup (e.g. restoring the DKMS transaction hook) belongs to
		# the whole installer, not to each individual step.
		trap 'installer_exit "$?"' EXIT
		"$@"
	)
	STEP_STATUS=$?
	set -e
	if (( STEP_STATUS != 0 )); then
		record_error "$label failed (status $STEP_STATUS); continuing with independent steps"
	fi
	return 0
}

info() {
	printf '[kait2en] %s\n' "$*"
}

warn() {
	printf '[kait2en] warning: %s\n' "$*" >&2
	printf '%s: warning: %s\n' "${0##*/}" "$*" >>"$KAIT2EN_INSTALL_ERRORS" ||
		printf '[kait2en] error: cannot write warning report\n' >&2
}

fail() {
	record_error "$*"
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

install_kait2en_fonts() {
	local font_source="$REPO_ROOT/assets/fonts"
	local font_dir=/usr/local/share/fonts/kait2en
	local license_dir=/usr/local/share/licenses/kait2en-fonts

	[[ -f "$font_source/JetBrainsMono-Regular.ttf" &&
		-f "$font_source/JetBrainsMono-Medium.ttf" &&
		-f "$font_source/OFL.txt" ]] || fail "bundled JetBrains Mono files are missing"
	install -d -o root -g root -m 0755 "$font_dir" "$license_dir"
	install -o root -g root -m 0644 \
		"$font_source/JetBrainsMono-Regular.ttf" \
		"$font_source/JetBrainsMono-Medium.ttf" \
		"$font_dir/"
	install -o root -g root -m 0644 "$font_source/OFL.txt" "$license_dir/OFL.txt"
	if command -v fc-cache >/dev/null 2>&1; then
		fc-cache -f "$font_dir" || warn "unable to refresh the font cache; continuing"
	else
		warn "fc-cache is unavailable; JetBrains Mono will appear after the next font-cache refresh"
	fi
}

kernel_release() {
	printf '%s\n' "${KERNEL_RELEASE:-$(uname -r)}"
}

require_kernel_headers() {
	local release
	# install_module calls dkms without -k, so the build always targets the
	# running kernel whatever KERNEL_RELEASE says.
	release="$(uname -r)"

	[[ -d "/lib/modules/$release/build" ]] ||
		fail "kernel-devel-$release is missing; run install-dependencies.sh first"
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
