#!/usr/bin/env bash

set -u

STATE_DIR="/run/kait2en-suspend"
LOG_TAG="kait2en-suspend"

log() {
	printf '[%s] %s\n' "$LOG_TAG" "$*"
}

is_loaded() {
	[[ -d "/sys/module/$1" ]]
}

try_unload() {
	local module=$1
	if ! is_loaded "$module"; then
		return 0
	fi

	log "unloading $module"
	if rmmod -f "$module"; then
		touch "$STATE_DIR/$module.unloaded"
	else
		log "could not unload $module"
	fi
}

try_load() {
	local module=$1
	[[ -e "$STATE_DIR/$module.unloaded" ]] || return 0

	log "loading $module"
	if ! modprobe "$module"; then
		log "could not load $module"
	fi
}

current_model() {
	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	cat /sys/class/dmi/id/product_name
}

needs_amdgpu_suspend_fix() {
	local model
	model="$(current_model 2>/dev/null || true)"

	case "$model" in
		MacBookPro15,1|MacBookPro15,3|MacBookPro16,1|MacBookPro16,4)
			is_loaded amdgpu
			;;
		*)
			return 1
			;;
	esac
}

has_bcm4377() {
	local dev device

	for dev in /sys/bus/pci/devices/*; do
		[[ -r "$dev/vendor" ]] || continue
		[[ "$(cat "$dev/vendor")" == "0x14e4" ]] || continue
		[[ -r "$dev/device" ]] || continue
		device="$(cat "$dev/device")"

		case "$device" in
			0x5f69|0x5f71|0x5f72|0x5fa0)
				return 0
				;;
		esac
	done

	return 1
}

pre_suspend() {
	mkdir -p "$STATE_DIR"
	rm -f "$STATE_DIR"/*.unloaded

	if needs_amdgpu_suspend_fix; then
		try_unload amdgpu
	else
		log "amdgpu suspend fix not needed"
	fi

	if has_bcm4377; then
		try_unload brcmfmac_wcc
		try_unload brcmfmac
		try_unload hci_bcm4377
	else
		log "BCM4377 suspend fix not needed"
	fi
}

post_resume() {
	try_load amdgpu

	try_load hci_bcm4377
	try_load brcmfmac
	try_load brcmfmac_wcc

	rm -f "$STATE_DIR"/*.unloaded
}

case "${1:-}" in
	pre)
		pre_suspend
		;;
	post)
		post_resume
		;;
	*)
		log "usage: $0 pre|post"
		exit 2
		;;
esac

exit 0
