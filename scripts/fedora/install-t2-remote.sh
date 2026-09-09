#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install nmcli rm systemctl readlink

readonly SERVICE="kait2en-t2-remote.service"
readonly CONNECTION="kait2en-t2-ncm"
readonly OBSOLETE_SERVICE="kait2en-t2-ncm-down.service"
readonly OBSOLETE_SERVICE_FILE="/etc/systemd/system/$OBSOLETE_SERVICE"
readonly OBSOLETE_HELPER="/usr/local/libexec/kait2en/kait2en-t2-ncm-down.sh"

[[ -x /usr/local/bin/t2remote ]] || fail "t2remote is not installed; run install-apps.sh first"

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
profile=""
best_rank=0
legacy_profiles=()

	uuids="$(nmcli --get-values UUID connection show)"
	for uuid in $uuids; do
		type="$(nmcli --get-values connection.type connection show uuid "$uuid")"
		[[ "$type" == 802-3-ethernet ]] || continue
		name="$(nmcli --get-values connection.id connection show uuid "$uuid")"
		bound_if="$(nmcli --get-values connection.interface-name connection show uuid "$uuid")"
		bound_mac="$(nmcli --get-values 802-3-ethernet.mac-address connection show uuid "$uuid")"
		rank=0
		if [[ "$name" == "$CONNECTION" ]]; then
			legacy_profiles+=("$uuid")
			rank=1
		elif [[ ( -n "$INTERFACE" && "$bound_if" == "$INTERFACE" ) ||
		        ( -n "$MAC" && "${bound_mac,,}" == "${MAC,,}" ) ]]; then
			rank=2
		fi
		[[ "$name" != "Apple T2 Bridge" ]] || rank=3
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
	/etc/NetworkManager/conf.d/99-network-t2-ncm.conf

# The OS installer prepares the next boot. Do not trigger udev, change live
# device management, or wait for carrier/IPv6 here.
install -o root -g root -m 0644 "$REPO_ROOT/systemd/$SERVICE" "/etc/systemd/system/$SERVICE"
systemctl daemon-reload
systemctl enable "$SERVICE"

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
nmcli connection modify uuid "$profile" \
	connection.id "Apple T2 Bridge" \
	"${binding[@]}" \
	connection.permissions "" \
	connection.autoconnect yes \
	connection.autoconnect-retries 0 \
	ipv4.method disabled ipv6.method link-local

# Only remove profiles created by the old installer, after the replacement
# profile has been saved successfully. An active connection may end here.
for uuid in "${legacy_profiles[@]}"; do
	[[ "$uuid" != "$profile" ]] || continue
	nmcli connection delete uuid "$uuid"
done
info "T2 profile $profile configured; automatic connection and service start on next boot"
