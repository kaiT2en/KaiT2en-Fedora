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

# All Rust projects build into one target directory in the invoking user's
# cache. The shared GTK stack compiles once and updates build incrementally.
if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != root ]]; then
	CARGO_TARGET_DIR="$(getent passwd "$SUDO_USER" | cut -d: -f6)/.cache/kait2en/cargo-target"
	export CARGO_TARGET_DIR
fi

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
	# The .hard file exists only when an error was recorded.
	local report
	if [[ ${EUID:-$(id -u)} -eq 0 && -n "${SUDO_USER:-}" ]]; then
		for report in "$KAIT2EN_INSTALL_ERRORS" "$KAIT2EN_INSTALL_HARD_ERRORS"; do
			[[ -e "$report" ]] || continue
			chown "$SUDO_USER" "$report" || printf '[kait2en] warning: cannot hand %s to %s\n' "$report" "$SUDO_USER" >&2
		done
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

# Records the commit a step or app was last installed from.
# kait2en-update compares it with GitHub to pick what has to run again.
KAIT2EN_APPLIED_DIR=${KAIT2EN_APPLIED_DIR:-/var/lib/kait2en-installer/applied}
mark_applied() {
	local name=$1 commit
	commit=$(git -c safe.directory="$REPO_ROOT" -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null) || return 0
	install -d -m 0755 "$KAIT2EN_APPLIED_DIR" &&
		printf '%s\n' "$commit" >"$KAIT2EN_APPLIED_DIR/$name" ||
		printf '[kait2en] warning: cannot record %s as installed\n' "$name" >&2
}

warn() {
	printf '[kait2en] warning: %s\n' "$*" >&2
	printf '%s: warning: %s\n' "${0##*/}" "$*" >>"$KAIT2EN_INSTALL_ERRORS" ||
		printf '[kait2en] error: cannot write warning report\n' >&2
}

# sudo resets the environment, so the shared target directory is passed on.
build_cargo_project() {
	local path=$1 target_user=$2
	sudo -H -u "$target_user" env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" make -C "$path" build
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

# Every T2 MacBook Pro has a Touch Bar; the MacBook Air, Mac mini, iMac and
# Mac Pro models do not.
has_touch_bar() {
	local model
	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	[[ "$model" == MacBookPro15,* || "$model" == MacBookPro16,* ]]
}

# Every T2 MacBook has a keyboard backlight, the desktop models do not.
has_keyboard_backlight() {
	local model
	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	[[ "$model" == MacBookPro15,* || "$model" == MacBookPro16,* || "$model" == MacBookAir8,* || "$model" == MacBookAir9,* ]]
}

# T2 MacBooks with the ambient light sensor behind the T2. iMacs have one
# too, but driving their panel is not verified yet.
has_ambient_light_sensor() {
	local model
	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	[[ "$model" == MacBookPro15,* || "$model" == MacBookPro16,* || "$model" == MacBookAir8,* || "$model" == MacBookAir9,* ]]
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

kernel_version_ge() {
	local release=$1 req_major=$2 req_minor=$3 req_patch=${4:-0}
	local major minor patch

	if [[ ! "$release" =~ ^([0-9]+)\.([0-9]+)(\.([0-9]+))? ]]; then
		return 1
	fi

	major="${BASH_REMATCH[1]}"
	minor="${BASH_REMATCH[2]}"
	patch="${BASH_REMATCH[4]:-0}"

	if (( major > req_major )); then
		return 0
	elif (( major == req_major && minor > req_minor )); then
		return 0
	elif (( major == req_major && minor == req_minor && patch >= req_patch )); then
		return 0
	fi
	return 1
}

# Unpacked and Fedora-patched kernel source per kernel release, shared by
# every step that rebuilds in-tree drivers, so the SRPM is downloaded once per
# kernel. Callers copy what they patch, the pool itself is never patched. A
# self-built kernel with a full source tree behind /lib/modules/<kver>/build
# is used directly. Prints the tree, messages go to stderr.
KERNEL_SOURCE_CACHE=/var/cache/kait2en/kernel-source

kernel_source_tree() {
	local kver=$1
	local pool="$KERNEL_SOURCE_CACHE/$kver"
	local build_tree staging source_rpm source_name source_version
	local source_release source_url tarball redhat_patch tree

	build_tree=$(readlink -f "/lib/modules/$kver/build" || true)
	if [[ -n "$build_tree" && -f "$build_tree/Makefile" && -f "$build_tree/init/main.c" ]]; then
		printf '%s\n' "$build_tree"
		return 0
	fi
	if [[ -f "$pool/.complete" && -d "$pool/linux" ]]; then
		printf '%s\n' "$pool/linux"
		return 0
	fi

	source_rpm=$(rpm -q --qf '%{SOURCERPM}\n' "kernel-core-$kver") || {
		record_error "kernel-core-$kver is not installed, no kernel source for it"
		return 1
	}
	source_name=${source_rpm%%-*}
	source_version=$(rpm -q --qf '%{VERSION}\n' "kernel-core-$kver")
	source_release=$(rpm -q --qf '%{RELEASE}\n' "kernel-core-$kver")
	source_url="https://kojipkgs.fedoraproject.org/packages/$source_name/$source_version/$source_release/src/$source_rpm"

	rm -rf "$pool" "$pool".partial.*
	staging="$pool.partial.$$"
	mkdir -p "$staging/srpm" "$staging/unpacked" || {
		record_error "unable to create $staging"
		return 1
	}

	info "downloading $source_rpm" >&2
	if ! curl --fail --location --silent --show-error \
			--output "$staging/$source_rpm" "$source_url"; then
		rm -rf "$staging"
		record_error "unable to download $source_url"
		return 1
	fi

	info "preparing the Fedora kernel sources for $kver" >&2
	if ! (cd "$staging/srpm" && rpm2cpio "../$source_rpm" | cpio -idm --quiet); then
		rm -rf "$staging"
		record_error "unable to unpack $source_rpm"
		return 1
	fi
	tarball=$(find "$staging/srpm" -maxdepth 1 -type f -name 'linux-*.tar.xz' -print -quit)
	redhat_patch=$(find "$staging/srpm" -maxdepth 1 -type f -name 'patch-*-redhat.patch' -print -quit)
	if [[ -z "$tarball" || -z "$redhat_patch" ]]; then
		rm -rf "$staging"
		record_error "$source_rpm lacks the upstream tarball or the Fedora patch"
		return 1
	fi
	if ! tar -xf "$tarball" -C "$staging/unpacked"; then
		rm -rf "$staging"
		record_error "unable to extract $tarball"
		return 1
	fi
	tree=$(find "$staging/unpacked" -mindepth 1 -maxdepth 1 -type d -print -quit)
	if [[ -z "$tree" ]] || ! git -C "$tree" apply "$redhat_patch"; then
		rm -rf "$staging"
		record_error "Fedora kernel patch does not apply to its upstream source"
		return 1
	fi

	mv "$tree" "$staging/linux"
	rm -rf "$staging/srpm" "$staging/unpacked" "${staging:?}/$source_rpm"
	touch "$staging/.complete"
	mv "$staging" "$pool"
	printf '%s\n' "$pool/linux"
}

# Drops the pooled sources of kernels that are no longer installed.
prune_kernel_sources() {
	local entry

	[[ -d "$KERNEL_SOURCE_CACHE" ]] || return 0
	for entry in "$KERNEL_SOURCE_CACHE"/*; do
		[[ -e "$entry" ]] || continue
		[[ -d "/usr/lib/modules/${entry##*/}" ]] || rm -rf "$entry"
	done
}

# Patch helpers for scripts that rebuild in-tree drivers. They expect
# PATCH_DIR, PATCH_SERIES, PATCH_FILES and KVER from the caller.
apply_patch_if_needed() {
	local tree=$1 patch_file=$2

	if patch -d "$tree" -p1 --dry-run --batch --forward --fuzz=3 \
			--no-backup-if-mismatch \
			<"$patch_file" >/dev/null 2>&1; then
		info "applying ${patch_file#"$REPO_ROOT/"}"
		patch -d "$tree" -p1 --batch --forward --fuzz=3 \
			--no-backup-if-mismatch <"$patch_file"
	elif patch -d "$tree" -p1 --dry-run --batch --reverse --fuzz=3 \
			--no-backup-if-mismatch \
			<"$patch_file" >/dev/null 2>&1; then
		info "${patch_file#"$REPO_ROOT/"} is already present"
	else
		fail "patch does not apply to $KVER: ${patch_file#"$REPO_ROOT/"}"
	fi
}

load_patch_series() {
	local entry

	[[ -r "$PATCH_SERIES" ]] || fail "patch series is missing: ${PATCH_SERIES#"$REPO_ROOT/"}"
	while IFS= read -r entry || [[ -n "$entry" ]]; do
		[[ -n "$entry" && "$entry" != \#* ]] || continue
		[[ "$entry" != */* && "$entry" != .* ]] ||
			fail "invalid patch series entry: $entry"
		[[ -f "$PATCH_DIR/$entry" ]] ||
			fail "patch listed in series is missing: $entry"
		PATCH_FILES+=("$PATCH_DIR/$entry")
	done <"$PATCH_SERIES"
	((${#PATCH_FILES[@]} > 0)) || fail "patch series is empty"
}
