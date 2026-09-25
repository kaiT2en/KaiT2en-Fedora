#!/usr/bin/env bash

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
	make -C "$REPO_ROOT/t2-services/shared" install SYSTEMD_UNIT_DIR=/etc/systemd/system INSTALL_NETWORK_PROFILE=no
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

	# Prefer Apple T2 Bridge even when the accidentally created profile is active.
	# Collect every profile that belongs to the NCM device so the duplicates
	# NetworkManager auto-creates can be removed, keeping only the best one.
	profile=""
	best_rank=0
	ncm_profiles=()

		uuids="$(nmcli --get-values UUID connection show)"
		for uuid in $uuids; do
			type="$(nmcli --get-values connection.type connection show uuid "$uuid")"
			[[ "$type" == 802-3-ethernet ]] || continue
			name="$(nmcli --get-values connection.id connection show uuid "$uuid")"
			bound_if="$(nmcli --get-values connection.interface-name connection show uuid "$uuid")"
			bound_mac="$(nmcli --get-values 802-3-ethernet.mac-address connection show uuid "$uuid")"
			rank=0
			if [[ "$name" == "Apple T2 Bridge" ]]; then
				rank=3
			elif [[ "$name" == "$CONNECTION" ]]; then
				rank=1
			elif [[ ( -n "$INTERFACE" && "$bound_if" == "$INTERFACE" ) ||
			        "$bound_if" == t2_ncm ||
			        ( -n "$MAC" && "${bound_mac,,}" == "${MAC,,}" ) ||
			        "${bound_mac,,}" == "$T2_NCM_MAC" ]]; then
				rank=2
			fi
			(( rank == 0 )) || ncm_profiles+=("$uuid")
			if (( rank > best_rank )); then
				profile="$uuid"
				best_rank=$rank
			fi
		done

	if [[ -e "$OBSOLETE_SERVICE_FILE" ]]; then
		info "removing obsolete T2 NCM shutdown service"
		systemctl disable "$OBSOLETE_SERVICE"
	fi
	rm -f "$OBSOLETE_SERVICE_FILE" "$OBSOLETE_HELPER"
	rm -f /etc/udev/rules.d/90-kait2en-t2-network.rules \
		/etc/udev/rules.d/90-kait2en-t2-network-managed.rules \
		/etc/NetworkManager/conf.d/99-network-t2-ncm.conf \
		/etc/NetworkManager/conf.d/10-kait2en-t2-no-auto-default.conf

	# Stop NetworkManager auto-creating a fresh generic profile every time the NCM
	# device re-enumerates. Without this it accumulates duplicate wired profiles.
	install -d -o root -g root -m 0755 /etc/NetworkManager/conf.d
	install -o root -g root -m 0644 \
		"$REPO_ROOT/t2-services/shared/integration/NetworkManager/10-t2-services.conf" \
		/etc/NetworkManager/conf.d/10-t2-services.conf

	# The OS installer prepares the next boot. Do not trigger udev, change live
	# device management, or wait for carrier/IPv6 here.
	# The shared integration has no dependency on the AVE or Touch ID daemons.

	if [[ -z "$profile" && -z "$MAC" ]]; then
		warn "T2 service installed; no NCM device or existing profile available to configure"
		exit 0
	fi

	if [[ -z "$profile" ]]; then
		info "creating IPv6 link-local profile for $INTERFACE"
		profile="$(< /proc/sys/kernel/random/uuid)"
		nmcli connection add type ethernet con-name "Apple T2 Bridge" \
			connection.uuid "$profile" ifname "*" \
			802-3-ethernet.mac-address "$MAC" \
			connection.autoconnect no ipv4.method disabled ipv6.method link-local
	fi

	binding=()
	if [[ -n "$MAC" ]]; then
		binding=(connection.interface-name "" 802-3-ethernet.mac-address "$MAC")
	fi
	# Normalize every autoconnect-related property: a profile left over from
	# an older mechanism (e.g. the retired t2_ncm udev rule) can carry a
	# stale autoconnect-priority that keeps NetworkManager from activating
	# it, and nothing else corrects that once t2-ncm-sleep stops nudging the
	# connection up on resume.
	nmcli connection modify uuid "$profile" \
		connection.id "Apple T2 Bridge" \
		"${binding[@]}" \
		connection.permissions "" \
		connection.autoconnect yes \
		connection.autoconnect-priority 0 \
		connection.autoconnect-retries 0 \
		ipv4.method disabled ipv6.method link-local

	# Remove the other NCM profiles (NetworkManager's auto-created generics and
	# old-name leftovers), after the replacement profile has been saved. An active
	# connection may end here.
	for uuid in "${ncm_profiles[@]}"; do
		[[ "$uuid" != "$profile" ]] || continue
		nmcli connection delete uuid "$uuid"
	done
	info "T2 profile $profile configured; automatic connection and service start on next boot"
}
run_step "configure T2 NCM network" configure_ncm
