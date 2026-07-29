#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_fedora
require_command dnf rpm grep sort

readonly AMD_VENDOR_ID="0x1002"
readonly INTEL_VENDOR_ID="0x8086"

package_installed() {
	rpm -q "$1" >/dev/null 2>&1
}

package_available() {
	[[ -n "$(dnf repoquery --quiet --cacheonly --assumeno --available "$1" 2>/dev/null)" ]]
}

# Prints the PCI vendor ID of every GPU exposed under /sys/class/drm, one per
# line. card* covers each GPU once, unlike the renderD*/controlD* nodes.
gpu_vendor_ids() {
	local card

	for card in /sys/class/drm/card[0-9]*; do
		[[ -r "$card/device/vendor" ]] || continue
		printf '%s\n' "$(<"$card/device/vendor")"
	done | sort -u
}

install_rpmfusion() {
	local fedora_version repo pkg url

	fedora_version="$(rpm -E %fedora)"

	for repo in free nonfree; do
		pkg="rpmfusion-$repo-release"
		if package_installed "$pkg"; then
			info "$pkg is already installed"
			continue
		fi
		url="https://mirrors.rpmfusion.org/$repo/fedora/${pkg}-${fedora_version}.noarch.rpm"
		info "enabling RPM Fusion $repo"
		dnf install -y "$url"
	done
}

# Replaces the "from" VA-API driver package with the "to" one. Falls
# back to a plain install when "from" is not installed, since dnf swap
# requires the source package to be present.
swap_or_install() {
	local from=$1 to=$2

	if package_installed "$to"; then
		info "$to is already installed"
		return 0
	fi

	if ! package_available "$to"; then
		warn "$to is not available in the enabled repositories; skipping it"
		return 0
	fi

	if package_installed "$from"; then
		dnf swap -y "$from" "$to"
	else
		dnf install -y "$to"
	fi
}

install_amd_drivers() {
	info "installing AMD VA-API driver (mesa-va-drivers-freeworld)"
	swap_or_install mesa-va-drivers mesa-va-drivers-freeworld
}

install_intel_drivers() {
	info "installing Intel VA-API driver (intel-media-driver)"
	swap_or_install libva-intel-media-driver intel-media-driver
}

# Checks whether $2 (vainfo output) lists an entrypoint matching $4 for any
# profile matching $3, and reports the result as $1: $5 available/missing.
check_capability() {
	local dev=$1 output=$2 profile_pattern=$3 entrypoint_pattern=$4 label=$5

	if grep -qE "${profile_pattern}[[:space:]]*:[[:space:]]*${entrypoint_pattern}" <<<"$output"; then
		info "$dev: $label available"
	else
		warn "$dev: $label not available"
	fi
}

verify_render_nodes() {
	local dev output

	for dev in /dev/dri/renderD*; do
		[[ -e "$dev" ]] || continue

		if ! output="$(vainfo --display drm --device "$dev" 2>&1)"; then
			warn "$dev: vainfo failed to query this render node"
			continue
		fi

		check_capability "$dev" "$output" 'VAProfileH264[A-Za-z]*' 'VAEntrypointVLD' "H.264 decoding"
		check_capability "$dev" "$output" 'VAProfileH264[A-Za-z]*' 'VAEntrypointEnc[A-Za-z]*' "H.264 encoding"
		check_capability "$dev" "$output" 'VAProfileVP9Profile[0-9]' 'VAEntrypointVLD' "VP9 decoding"
		check_capability "$dev" "$output" 'VAProfileVP9Profile[0-9]' 'VAEntrypointEnc[A-Za-z]*' "VP9 encoding"
	done
}

info "installing hardware video decoding support"

install_rpmfusion
dnf makecache --refresh
dnf install -y libva-utils

found_amd=
found_intel=
while IFS= read -r vendor; do
	case "$vendor" in
		"$AMD_VENDOR_ID")
			found_amd=1
			install_amd_drivers
			;;
		"$INTEL_VENDOR_ID")
			found_intel=1
			install_intel_drivers
			;;
	esac
done < <(gpu_vendor_ids)

[[ -n "$found_amd$found_intel" ]] ||
	fail "no AMD or Intel GPU found under /sys/class/drm"

verify_render_nodes

info "hardware video decoding drivers installed"
info "see howto/postinstall/hardware-video-decoding.md for further details"
