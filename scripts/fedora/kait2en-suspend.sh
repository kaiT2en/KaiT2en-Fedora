#!/usr/bin/env bash

set -uo pipefail

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
	if ! touch "$STATE_DIR/$module.unloaded"; then
		log "could not prepare the state file for $module; leaving it loaded"
		return 1
	fi

	if ! rmmod -f "$module"; then
		log "could not unload $module"
		if ! rm -f "$STATE_DIR/$module.unloaded"; then
			log "could not remove the unused state file for $module"
		fi
		return 1
	fi
}

try_load() {
	local module=$1
	[[ -e "$STATE_DIR/$module.unloaded" ]] || return 0

	log "loading $module"
	if ! modprobe "$module"; then
		log "could not load $module"
		return 1
	fi

	if ! rm -f "$STATE_DIR/$module.unloaded"; then
		log "could not remove the state file for $module"
		return 1
	fi
}

restore_unloaded_modules() {
	if ! try_load brcmfmac; then
		log "continuing after brcmfmac could not be restored"
	fi
	if ! try_load brcmfmac_wcc; then
		log "continuing after brcmfmac_wcc could not be restored"
	fi

	if [[ -e "$STATE_DIR/hci_bcm4377.unloaded" ]]; then
		log "waiting 5 seconds before loading hci_bcm4377"
		sleep 5
	fi
	if ! try_load hci_bcm4377; then
		log "continuing after hci_bcm4377 could not be restored"
	fi
}

# Succeeds when a Broadcom PCI function with one of the given device IDs is
# present. Returns 2 when sysfs could not be read.
has_broadcom_device() {
	local dev vendor device id

	for dev in /sys/bus/pci/devices/*; do
		[[ -r "$dev/vendor" ]] || continue
		if ! vendor="$(cat "$dev/vendor")"; then
			log "could not read PCI vendor from $dev"
			return 2
		fi
		[[ "$vendor" == "0x14e4" ]] || continue
		if [[ ! -r "$dev/device" ]]; then
			log "could not access PCI device ID in $dev"
			return 2
		fi
		if ! device="$(cat "$dev/device")"; then
			log "could not read PCI device ID from $dev"
			return 2
		fi

		for id in "$@"; do
			[[ "$device" == "$id" ]] && return 0
		done
	done

	return 1
}

unload_brcmfmac() {
	if ! try_unload brcmfmac_wcc; then
		log "continuing suspend after brcmfmac_wcc could not be unloaded"
	fi
	if ! try_unload brcmfmac; then
		log "continuing suspend after brcmfmac could not be unloaded"
	fi
}

pre_suspend() {
	local status

	if ! mkdir -p "$STATE_DIR"; then
		log "could not create state directory $STATE_DIR; skipping suspend fixes"
		return 0
	fi

	has_broadcom_device 0x5f69 0x5f71 0x5f72 0x5fa0
	status=$?
	case "$status" in
		0)
			unload_brcmfmac
			if ! try_unload hci_bcm4377; then
				log "continuing suspend after hci_bcm4377 could not be unloaded"
			fi
			return 0
			;;
		1)
			;;
		*)
			log "BCM4377 detection failed; skipping its suspend fix"
			return 0
			;;
	esac

	# BCM4350 in the MacBookPro14,1 (A1708) stops answering firmware commands
	# after S3 and stays dead across a warm reboot. Its Bluetooth uses UART, so
	# only the WLAN driver needs to go.
	has_broadcom_device 0x43a3
	status=$?
	case "$status" in
		0)
			unload_brcmfmac
			;;
		1)
			log "Broadcom WLAN suspend fix not needed"
			;;
		*)
			log "BCM4350 detection failed; skipping its suspend fix"
			;;
	esac

	return 0
}

post_resume() {
	restore_unloaded_modules
	return 0
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
