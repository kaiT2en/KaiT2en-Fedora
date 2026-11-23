#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
#
# Temporarily hand the T2 actuator HID interface from t2hid to
# t2_trackpad_actuator, then restore it unconditionally after DURATION
# seconds (default 10, override with the DURATION env var). Keep this
# bounded and automatic: see t2_precision_trackpad/test-handoff.sh for the
# prior incident that makes this non-negotiable.
#
# Unlike the trackpad handoff, this checks that the new driver binds to
# exactly one of the five sibling HID interfaces on the same composite USB
# device (the vendor usage-page/usage match in probe()), not the whole
# device. The other four are also checked to remain untouched.

set -u

new_driver=t2_trackpad_actuator
new_module=t2_trackpad_actuator
old_driver=t2hid
duration=${DURATION:-10}
module_path="$(dirname "$0")/t2_trackpad_actuator.ko"
device=
original_driver=
swapped=0

die()
{
	echo "error: $*" >&2
	exit 1
}

driver_name()
{
	local path=$1

	[ -L "$path/driver" ] || return 1
	basename "$(readlink "$path/driver")"
}

restore()
{
	local current

	[ "$swapped" = 1 ] || return 0
	trap - EXIT HUP INT TERM
	set +e

	current=$(driver_name "/sys/bus/hid/devices/$device")
	if [ "$current" = "$new_driver" ]; then
		echo "$device" > "/sys/bus/hid/drivers/$new_driver/unbind"
	fi

	/sbin/rmmod "$new_module"
	current=$(driver_name "/sys/bus/hid/devices/$device")
	if [ -z "$current" ] && [ "$original_driver" = "$old_driver" ]; then
		echo "$device" > "/sys/bus/hid/drivers/$old_driver/bind"
	fi

	current=$(driver_name "/sys/bus/hid/devices/$device")
	if [ "$current" = "$original_driver" ]; then
		echo "restored $device to $original_driver"
	else
		echo "CRITICAL: $device is bound to ${current:-no driver}" >&2
	fi
}

[ "$(id -u)" -eq 0 ] || die "run as root"
[ -r "$module_path" ] || die "module not built: $module_path"
case "$duration" in
''|*[!0-9]*|0) die "DURATION must be a positive integer" ;;
esac
[ "$duration" -le 120 ] || die "DURATION must stay bounded (<=120s) -- this is an automatic-rollback test, not a persistent bind"

# Find every 0003:05AC:0280.* interface up front, for the before/after check.
all_before=()
for path in /sys/bus/hid/devices/0003:05AC:0280.*; do
	[ -e "$path" ] || continue
	all_before+=("${path##*/}=$(driver_name "$path")")
done
[ "${#all_before[@]}" -gt 0 ] || die "no T2 keyboard/trackpad HID interfaces found"

# Several sibling interfaces on this composite device are bound to $old_driver
# (keyboard, this actuator, and others). Pick the one whose report descriptor
# declares the vendor usage-page 0xff00 / usage 0x0d application collection.
# This is the exact match t2_trackpad_actuator's probe() uses.
for path in /sys/bus/hid/devices/0003:05AC:0280.*; do
	[ -e "$path" ] || continue
	current=$(driver_name "$path")
	[ "$current" = "$old_driver" ] || continue
	[ -r "$path/report_descriptor" ] || continue
	if od -An -tx1 "$path/report_descriptor" | tr -d ' \n' | grep -qi '0600ff090da101'; then
		[ -z "$device" ] || die "more than one interface matches the actuator usage; refusing to guess"
		device=${path##*/}
		original_driver=$current
	fi
done

[ -n "$device" ] || die "no $old_driver interface with usage-page 0xff00/usage 0x0d found (already swapped, or driver name changed)"

trap restore EXIT HUP INT TERM

swapped=1
echo "$device" > "/sys/bus/hid/drivers/$old_driver/unbind" ||
	die "could not unbind $device from $old_driver"

/sbin/insmod "$module_path" || die "could not load $new_module"

if [ -z "$(driver_name "/sys/bus/hid/devices/$device")" ]; then
	echo "$device" > "/sys/bus/hid/drivers/$new_driver/bind" ||
		die "could not bind $device to $new_driver"
fi

if [ "$(driver_name "/sys/bus/hid/devices/$device")" != "$new_driver" ]; then
	die "new driver did not bind to $device -- check the usage-page/usage match in probe()"
fi

echo "bound $device to $new_driver"

test_fire_param="/sys/module/$new_module/parameters/test_fire"
if [ -w "$test_fire_param" ]; then
	echo "firing test waveform 9 (all-bytes-maxed diagnostic probe) via $test_fire_param"
	echo 9 > "$test_fire_param" || echo "warning: write to test_fire failed" >&2
	sleep 1
else
	echo "warning: $test_fire_param not writable, skipping automatic test fire" >&2
fi

echo "sleeping ${duration}s"
sleep "$duration"

echo "driver bindings across all T2 keyboard/trackpad interfaces during the test:"
for path in /sys/bus/hid/devices/0003:05AC:0280.*; do
	[ -e "$path" ] || continue
	echo "  ${path##*/} -> $(driver_name "$path")"
done

echo "kernel log from this run:"
dmesg | grep -i "$new_module" | tail -20
