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

has_bcm4377() {
	local dev vendor device

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

		case "$device" in
			0x5f69|0x5f71|0x5f72|0x5fa0)
				return 0
				;;
		esac
	done

	return 1
}

is_t2_ncm_control() {
	local dev parent
	dev="$(readlink -f -- "$1")" || return 1
	parent="${dev%/*}"
	[[ -r "$parent/idVendor" && -r "$parent/idProduct" &&
	   -r "$dev/bInterfaceClass" && -r "$dev/bInterfaceSubClass" ]] || return 1
	[[ "$(<"$parent/idVendor")" == "05ac" &&
	   "$(<"$parent/idProduct")" == "8233" &&
	   "$(<"$dev/bInterfaceClass")" == "02" &&
	   "$(<"$dev/bInterfaceSubClass")" == "0d" ]]
}

unbind_t2_ncm() {
	local dev id net
	for dev in /sys/bus/usb/drivers/cdc_ncm/*:*; do
		is_t2_ncm_control "$dev" || continue
		id="${dev##*/}"
		# Save the USB control interface before its netdev disappears.
		if ! printf '%s\n' "$id" >>"$STATE_DIR/t2-ncm.interfaces"; then
			log "could not save NCM interface $id; leaving it bound"
			continue
		fi
		if command -v nmcli >/dev/null 2>&1; then
			for net in "$dev"/net/*; do
				[[ -e "$net" ]] || continue
				nmcli --wait 5 device disconnect "${net##*/}" >/dev/null 2>&1 || true
			done
		fi
		log "unbinding T2 CDC-NCM control interface $id"
		if ! printf '%s' "$id" >/sys/bus/usb/drivers/cdc_ncm/unbind; then
			log "could not unbind T2 CDC-NCM interface $id"
		fi
	done
}

bind_t2_ncm() {
	local id dev driver failed=0
	[[ -f "$STATE_DIR/t2-ncm.interfaces" ]] || return 0
	while IFS= read -r id; do
		[[ "$id" =~ ^[0-9]+-[0-9]+(\.[0-9]+)*:[0-9]+\.[0-9]+$ ]] || {
			log "invalid saved NCM interface: $id"
			failed=1
			continue
		}
		dev="/sys/bus/usb/devices/$id"
		if [[ ! -e "$dev" ]]; then
			# USB re-enumeration creates a new, automatically probed interface.
			log "NCM interface $id disappeared; waiting for USB re-enumeration"
			continue
		fi
		if ! is_t2_ncm_control "$dev"; then
			log "saved interface $id is not T2 NCM; refusing to bind it"
			failed=1
			continue
		fi
		if [[ -L "$dev/driver" ]]; then
			driver="$(readlink -f -- "$dev/driver")"
			if [[ "${driver##*/}" != cdc_ncm ]]; then
				log "interface $id is owned by ${driver##*/}; leaving it alone"
				failed=1
			fi
			continue
		fi
		log "binding T2 CDC-NCM control interface $id"
		if ! printf '%s' "$id" >/sys/bus/usb/drivers/cdc_ncm/bind; then
			log "could not bind T2 CDC-NCM interface $id"
			failed=1
		fi
	done <"$STATE_DIR/t2-ncm.interfaces"
	if (( failed == 0 )); then
		rm -f -- "$STATE_DIR/t2-ncm.interfaces"
	fi
	return "$failed"
}

pre_suspend() {
	local status

	if ! mkdir -p "$STATE_DIR"; then
		log "could not create state directory $STATE_DIR; skipping suspend fixes"
		return 0
	fi

	if [[ -S /run/t2remote.sock ]] && command -v t2remote >/dev/null 2>&1; then
		log "closing T2 RemoteXPC services and disconnecting NCM"
		if ! timeout --kill-after=2 15 t2remote pre-suspend; then
			log "T2 RemoteXPC pre-suspend failed; continuing suspend"
		fi
	fi

	unbind_t2_ncm

	has_bcm4377
	status=$?
	case "$status" in
		0)
			if ! try_unload brcmfmac_wcc; then
				log "continuing suspend after brcmfmac_wcc could not be unloaded"
			fi
			if ! try_unload brcmfmac; then
				log "continuing suspend after brcmfmac could not be unloaded"
			fi
			if ! try_unload hci_bcm4377; then
				log "continuing suspend after hci_bcm4377 could not be unloaded"
			fi
			;;
		1)
			log "BCM4377 suspend fix not needed"
			;;
		*)
			log "BCM4377 detection failed; skipping its suspend fix"
			;;
	esac

	return 0
}

post_resume() {
	bind_t2_ncm || log "T2 NCM rebind incomplete; saved interfaces retained for retry"
	restore_unloaded_modules
	if [[ -S /run/t2remote.sock ]] && command -v t2remote >/dev/null 2>&1; then
		log "reconnecting NCM and requested T2 RemoteXPC services"
		if ! timeout --kill-after=2 120 t2remote post-resume; then
			log "T2 RemoteXPC post-resume failed; services can be retried manually"
		fi
	fi
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
