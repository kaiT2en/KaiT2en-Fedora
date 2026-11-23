#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install nmcli rm systemctl readlink

readonly CONNECTION="kait2en-t2-ncm"
readonly OBSOLETE_SERVICE="kait2en-t2-ncm-down.service"
readonly OBSOLETE_SERVICE_FILE="/etc/systemd/system/$OBSOLETE_SERVICE"
readonly OBSOLETE_HELPER="/usr/local/libexec/kait2en/kait2en-t2-ncm-down.sh"
# Fixed address of the T2 bridge CDC-NCM interface, the same on every T2 Mac.
readonly T2_NCM_MAC="ac:de:48:00:11:22"

install_shared() {
	make -C "$REPO_ROOT/t2-services/shared" install SYSTEMD_UNIT_DIR=/etc/systemd/system
	python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-services-common record-source
	if [[ -f /usr/local/libexec/kait2en/kait2en-suspend.sh ]] &&
		grep -q '^unbind_t2_ncm()' /usr/local/libexec/kait2en/kait2en-suspend.sh; then
		fail "old suspend helper still owns NCM; rerun install-suspend-service.sh before enabling the common unit"
	fi
	systemctl daemon-reload
	systemctl enable t2-services-suspend.service
}
run_step "install shared T2 integration" install_shared

configure_ncm() {
	# Match the same driver and USB device as discovery::interface() in t2remote.
	interfaces=()
	for net in /sys/class/net/*; do
		[[ -e "$net/device/driver" ]] || continue
		driver="$(readlink -f "$net/device/driver")"
		[[ "${driver##*/}" == cdc_ncm ]] || continue
		parent="$(readlink -f "$net/device")"
		while [[ "$parent" != / ]]; do
			if [[ -r "$parent/idVendor" && -r "$parent/idProduct" &&
			      "$(<"$parent/idVendor")" == 05ac && "$(<"$parent/idProduct")" == 8233 ]]; then
				interfaces+=("${net##*/}")
				break
			fi
			parent="${parent%/*}"
			[[ -n "$parent" ]] || break
		done
	done
	(( ${#interfaces[@]} <= 1 )) || fail "multiple Apple CDC-NCM interfaces found; refusing ambiguous profile changes"
	readonly INTERFACE="${interfaces[0]:-}"
	MAC=""
	if [[ -n "$INTERFACE" ]]; then
		MAC="$(<"/sys/class/net/$INTERFACE/address")"
	fi

	if [[ -e "$OBSOLETE_SERVICE_FILE" ]]; then
		info "removing obsolete T2 NCM shutdown service"
		systemctl disable "$OBSOLETE_SERVICE"
	fi
	rm -f "$OBSOLETE_SERVICE_FILE" "$OBSOLETE_HELPER"
	rm -f /etc/udev/rules.d/90-kait2en-t2-network.rules \
		/etc/udev/rules.d/90-kait2en-t2-network-managed.rules \
		/etc/NetworkManager/conf.d/99-network-t2-ncm.conf \
		/etc/NetworkManager/conf.d/10-kait2en-t2-no-auto-default.conf

	# NetworkManager leaves the NCM link unmanaged (10-t2-services.conf) and
	# 90-t2-services-ncm.rules brings it up. Otherwise NetworkManager takes the
	# link down on PrepareForSleep, before the suspend hooks can close their
	# T2 sessions, and bridgeOS drops SideCar while the host sleeps.
	install -d -o root -g root -m 0755 /etc/NetworkManager/conf.d
	install -o root -g root -m 0644 \
		"$REPO_ROOT/t2-services/shared/integration/NetworkManager/10-t2-services.conf" \
		/etc/NetworkManager/conf.d/10-t2-services.conf

	# Remove every profile of the NCM device, the former Apple T2 Bridge profile
	# and the duplicates NetworkManager auto-created, whatever their file name.
	# Unescaped, so a path containing ':' or '\' stays usable. UUIDs never
	# contain ':', so stripping the "uuid:" prefix leaves the exact path.
	filenames="$(nmcli --escape no --get-values UUID,FILENAME connection show)"
	uuids="$(nmcli --get-values UUID connection show)"
	for uuid in $uuids; do
		type="$(nmcli --get-values connection.type connection show uuid "$uuid")"
		[[ "$type" == 802-3-ethernet ]] || continue
		name="$(nmcli --get-values connection.id connection show uuid "$uuid")"
		bound_if="$(nmcli --get-values connection.interface-name connection show uuid "$uuid")"
		bound_mac="$(nmcli --get-values 802-3-ethernet.mac-address connection show uuid "$uuid")"
		[[ "$name" == "Apple T2 Bridge" || "$name" == "$CONNECTION" ||
		   ( -n "$INTERFACE" && "$bound_if" == "$INTERFACE" ) ||
		   "$bound_if" == t2_ncm ||
		   ( -n "$MAC" && "${bound_mac,,}" == "${MAC,,}" ) ||
		   "${bound_mac,,}" == "$T2_NCM_MAC" ]] || continue
		file=""
		while IFS= read -r line; do
			if [[ "$line" == "$uuid:"* ]]; then
				file="${line#"$uuid:"}"
				break
			fi
		done <<<"$filenames"
		# Delete only the file. The live connection keeps running until the
		# next boot, when NetworkManager no longer manages the device.
		if [[ "$file" == /etc/NetworkManager/system-connections/*.nmconnection ]]; then
			# A rerun before the reboot still lists the profile from memory.
			if [[ -f "$file" ]]; then
				info "removing T2 NCM profile $name ($file)"
				rm -- "$file"
			else
				info "T2 NCM profile $name already removed on disk, active until the next boot"
			fi
		elif [[ "$file" == /run/* ]]; then
			: # Runtime profiles disappear on reboot.
		else
			warn "T2 NCM profile $name ($uuid) not removed, file ${file:-unknown}; after the next boot run nmcli connection delete uuid $uuid"
		fi
	done
	info "T2 NCM link is unmanaged by NetworkManager from the next boot"
}
run_step "configure T2 NCM network" configure_ncm
