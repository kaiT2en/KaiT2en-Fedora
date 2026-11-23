#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
#
# Temporarily hand the one T2 HID mouse interface from hid_t2magicmouse to
# t2_precision_trackpad, then restore it unconditionally after DURATION seconds
# (default 10, override with the DURATION env var). Keep this bounded and
# automatic: an earlier manual unbind/insmod without an automatic rollback
# hung the shared T2 HID/VHCI transport and took keyboard input down with it
# (no external input device was attached), requiring a hard reboot.

set -u

new_driver=t2_precision_trackpad
new_module=t2_precision_trackpad
old_driver=t2magicmouse
duration=${DURATION:-10}
mode=${1:-all}
force_trace_frames=${FORCE_TRACE_FRAMES:-0}
force_trace_trigger_pressure=${FORCE_TRACE_TRIGGER_PRESSURE:-64}
module_path="$(dirname "$0")/t2_precision_trackpad.ko"
device=
original_driver=
swapped=0
capture_pid=
libinput_capture_pid=

case "$mode" in
all)
	instructions="move one, two, then three fingers and make a normal click"
	event_filter='ABS_MT_|BTN_(LEFT|MOUSE)'
	;;
touch)
	instructions="move one finger, then two fingers, then three fingers"
	event_filter='ABS_MT_(SLOT|TRACKING_ID|POSITION_)'
	;;
click)
	instructions="hold one finger still, then perform one normal Force Touch click"
	event_filter='ABS_MT_POSITION_|BTN_(LEFT|MOUSE)'
	;;
*)
	echo "usage: $0 [all|touch|click]" >&2
	exit 2
	;;
esac

capture_file=$(mktemp "/tmp/t2-precision-trackpad-${mode}.XXXXXX")
chmod 0644 "$capture_file"
libinput_capture_file=$(mktemp "/tmp/t2-precision-trackpad-libinput-${mode}.XXXXXX")
chmod 0644 "$libinput_capture_file"

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

stop_capture()
{
	[ -n "$capture_pid" ] || return 0
	kill "$capture_pid" 2>/dev/null
	wait "$capture_pid" 2>/dev/null
	capture_pid=
}

stop_libinput_capture()
{
	[ -n "$libinput_capture_pid" ] || return 0
	kill "$libinput_capture_pid" 2>/dev/null
	wait "$libinput_capture_pid" 2>/dev/null
	libinput_capture_pid=
}

restore()
{
	local current

	[ "$swapped" = 1 ] || return 0
	trap - EXIT HUP INT TERM
	set +e
	stop_capture
	stop_libinput_capture

	current=$(driver_name "/sys/bus/hid/devices/$device")
	if [ "$current" = "$new_driver" ]; then
		echo "$device" > "/sys/bus/hid/drivers/$new_driver/unbind"
	fi

	/sbin/rmmod "$new_module"
	current=$(driver_name "/sys/bus/hid/devices/$device")
	if [ -z "$current" ]; then
		if [ "$original_driver" = "$old_driver" ]; then
			echo "$device" > "/sys/bus/hid/drivers/$old_driver/bind"
		else
			/sbin/insmod "$module_path"
			# insmod normally autoprobes the unbound HID device. Only bind
			# explicitly if that did not happen. Otherwise sysfs returns EBUSY.
			if [ -z "$(driver_name "/sys/bus/hid/devices/$device")" ]; then
				echo "$device" > "/sys/bus/hid/drivers/$new_driver/bind"
			fi
		fi
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
case "$force_trace_frames" in
''|*[!0-9]*) die "FORCE_TRACE_FRAMES must be a non-negative integer" ;;
esac
case "$force_trace_trigger_pressure" in
''|*[!0-9]*) die "FORCE_TRACE_TRIGGER_PRESSURE must be a non-negative integer" ;;
esac

for path in /sys/bus/hid/devices/0003:05AC:0280.*; do
	[ -e "$path" ] || continue
	current=$(driver_name "$path")
	if [ "$current" = "$old_driver" ] || [ "$current" = "$new_driver" ]; then
		[ -z "$device" ] || die "more than one $old_driver T2 HID interface"
		device=${path##*/}
		original_driver=$current
	fi
done

[ -n "$device" ] || die "no T2 trackpad bound to $old_driver or $new_driver"

trap restore EXIT HUP INT TERM

swapped=1
if [ "$original_driver" = "$new_driver" ]; then
	echo "$device" > "/sys/bus/hid/drivers/$new_driver/unbind" ||
		die "could not unbind $device from $new_driver"
	/sbin/rmmod "$new_module" || die "could not unload $new_module"
else
	echo "$device" > "/sys/bus/hid/drivers/$old_driver/unbind" ||
		die "could not unbind $device from $old_driver"
fi

module_args=()
if [ "$force_trace_frames" -gt 0 ]; then
	module_args+=("force_trace_frames=$force_trace_frames")
	module_args+=("force_trace_trigger_pressure=$force_trace_trigger_pressure")
fi
/sbin/insmod "$module_path" "${module_args[@]}"

# Loading the driver normally triggers HID autoprobe.  Bind explicitly only
# when autoprobe left this interface unbound. A second bind returns EBUSY.
if [ -z "$(driver_name "/sys/bus/hid/devices/$device")" ]; then
	echo "$device" > "/sys/bus/hid/drivers/$new_driver/bind" ||
		die "could not bind $device to $new_driver"
fi

if [ "$(driver_name "/sys/bus/hid/devices/$device")" != "$new_driver" ]; then
	die "new driver did not bind"
fi

event_node=
for event_path in "/sys/bus/hid/devices/$device/input"/input*/event*; do
	[ -d "$event_path" ] || continue
	name_file="${event_path%/event*}/name"
	[ -r "$name_file" ] || continue
	if [ "$(cat "$name_file")" != "T2 Force Click Events" ]; then
		event_node=/dev/input/${event_path##*/}
		break
	fi
done
[ -n "$event_node" ] || die "could not find the trackpad event device"

echo "recording $event_node for $duration seconds; $instructions"
/usr/bin/timeout --signal=INT "$duration" /usr/bin/evtest "$event_node" > "$capture_file" 2>&1 &
capture_pid=$!
/usr/bin/timeout --signal=INT "$duration" /usr/bin/libinput debug-events --device "$event_node" > "$libinput_capture_file" 2>&1 &
libinput_capture_pid=$!
sleep "$duration"
wait "$capture_pid" 2>/dev/null
capture_pid=
wait "$libinput_capture_pid" 2>/dev/null
libinput_capture_pid=

echo "captured multitouch and button events:"
rg "$event_filter" "$capture_file" || true
echo "full capture retained at $capture_file"
echo "libinput touch, motion, and button events:"
rg 'TOUCH_(DOWN|MOTION|UP)|POINTER_(MOTION|BUTTON)' "$libinput_capture_file" || true
echo "full libinput capture retained at $libinput_capture_file"
