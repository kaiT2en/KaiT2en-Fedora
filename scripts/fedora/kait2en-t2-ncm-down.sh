#!/usr/bin/env bash

set -u

LOG_TAG="kait2en-t2-ncm"
TARGET_IFACE="t2_ncm"
WAIT_ATTEMPTS=40
WAIT_INTERVAL_SECS="0.25"

log() {
	printf '[%s] %s\n' "$LOG_TAG" "$*"
}

interface_exists() {
	[[ -e "/sys/class/net/$1" ]]
}

wait_for_interface() {
	local iface=$1
	local attempt

	for attempt in $(seq 1 "$WAIT_ATTEMPTS"); do
		if interface_exists "$iface"; then
			return 0
		fi
		sleep "$WAIT_INTERVAL_SECS"
	done

	return 1
}

interface_is_up() {
	local iface=$1

	ip -o link show dev "$iface" 2>/dev/null | grep -q '<[^>]*UP[,>]'
}

ensure_down_once() {
	local iface=$1

	log "setting $iface down"
	if ! ip link set dev "$iface" down; then
		log "could not set $iface down"
		return 1
	fi

	return 0
}

ensure_down_stable() {
	local iface=$1
	local attempt

	if ! wait_for_interface "$iface"; then
		log "$iface did not appear in time"
		return 0
	fi

	for attempt in $(seq 1 "$WAIT_ATTEMPTS"); do
		if interface_is_up "$iface"; then
			ensure_down_once "$iface" || true
		fi
		sleep "$WAIT_INTERVAL_SECS"
	done

	if interface_is_up "$iface"; then
		log "$iface is still up after retries"
		return 1
	fi

	log "$iface is down"
	return 0
}

main() {
	ensure_down_stable "$TARGET_IFACE"
}

main
