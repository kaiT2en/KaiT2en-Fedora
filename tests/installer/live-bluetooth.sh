#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
script="$repo_root/packaging/installer/runtime/kait2en-live-bluetooth"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-live-bt-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

# Every case gets its own fake sysfs, its own stubbed commands and a helper that
# only records that it ran. Nothing here touches the machine running the tests.
setup_case() {
	local name=$1 identifier=${2:-0x5fa0}
	local case_dir="$work/$name"

	mkdir -p "$case_dir/bin" "$case_dir/pci/0000:03:00.0" \
		"$case_dir/bluetooth" "$case_dir/source"
	printf '0x14e4\n' >"$case_dir/pci/0000:03:00.0/vendor"
	printf '%s\n' "$identifier" >"$case_dir/pci/0000:03:00.0/device"

	printf '%s\n' '#!/bin/sh' 'exit 0' >"$case_dir/bin/journalctl"
	printf '%s\n' '#!/bin/sh' 'exit 0' >"$case_dir/bin/dmesg"
	printf '%s\n' '#!/bin/sh' 'echo "modprobe $*" >>"$KAIT2EN_TEST_LOG"' \
		>"$case_dir/bin/modprobe"
	printf '%s\n' '#!/bin/sh' 'exit 0' >"$case_dir/bin/rfkill"
	printf '%s\n' '#!/bin/sh' 'exit 0' >"$case_dir/bin/systemctl"
	printf '%s\n' '#!/bin/sh' 'echo "helper $*" >>"$KAIT2EN_TEST_LOG"' \
		>"$case_dir/helper.sh"
	chmod 0755 "$case_dir/bin"/* "$case_dir/helper.sh"
	: >"$case_dir/log"
}

set_kernel_log() {
	local case_dir=$1
	shift
	{
		printf '%s\n' '#!/bin/sh'
		printf 'echo "%s"\n' "$@"
	} >"$case_dir/bin/dmesg"
	chmod 0755 "$case_dir/bin/dmesg"
}

unable_line() {
	printf "hci_bcm4377 0000:03:00.0: Unable to load firmware; tried '%s' and '%s'" \
		"$1" "$2"
}

run_live() {
	local case_dir=$1
	env \
		PATH="$case_dir/bin:$PATH" \
		KAIT2EN_TEST_LOG="$case_dir/log" \
		KAIT2EN_PCI_ROOT="$case_dir/pci" \
		KAIT2EN_LIVE_BT_ROOT="$case_dir/bluetooth" \
		KAIT2EN_LIVE_BT_SOURCE="$case_dir/source" \
		KAIT2EN_BT_HELPER="$case_dir/helper.sh" \
		KAIT2EN_LIVE_ROOT="$case_dir" \
		KAIT2EN_LIVE_BT_REQUEST_TIMEOUT=0 \
		KAIT2EN_LIVE_BT_LINK_TIMEOUT=0 \
		bash "$script"
}

# The firmware is missing, the driver says so, and a controller shows up once it
# has been installed.
setup_case ready
set_kernel_log "$work/ready" \
	"$(unable_line 'brcm/brcmbt4377b3-apple,formosa-u.bin' \
		'brcm/brcmbt4377b3-apple,formosa.bin')"
touch "$work/ready/bluetooth/hci0"
output=$(run_live "$work/ready")
grep -Fq 'helper --source' "$work/ready/log"
grep -Fq 'Bluetooth is ready in the live session: hci0' <<<"$output"

# A Mac with the older combo chip must be left alone entirely.
setup_case other 0x4464
output=$(run_live "$work/other")
grep -Fq 'no BCM4377 Bluetooth controller' <<<"$output"
[[ ! -s "$work/other/log" ]]

# Nothing to do once the driver is bound: the firmware is already in place.
setup_case bound
ln -s /sys/bus/pci/drivers/hci_bcm4377 "$work/bound/pci/0000:03:00.0/driver"
output=$(run_live "$work/bound")
grep -Fq 'already running' <<<"$output"
! grep -Fq 'helper' "$work/bound/log"

# Without a board type the driver never asks for firmware. Installing one would
# only hide that the kernel does not know this Mac.
setup_case unknown
set_kernel_log "$work/unknown" \
	'hci_bcm4377 0000:03:00.0: unable to determine board type'
output=$(run_live "$work/unknown")
grep -Fq 'does not know this Mac' <<<"$output"
! grep -Fq 'helper' "$work/unknown/log"

# A quiet log and an unbound driver is nothing this script can act on.
setup_case quiet
output=$(run_live "$work/quiet")
grep -Fq 'asked for no firmware' <<<"$output"
! grep -Fq 'helper' "$work/quiet/log"

# The firmware is wanted but the stick carries none: say so and change nothing.
setup_case nosource
set_kernel_log "$work/nosource" \
	"$(unable_line 'brcm/brcmbt4377b3-apple,formosa-u.bin' \
		'brcm/brcmbt4377b3-apple,formosa.bin')"
rmdir "$work/nosource/source"
output=$(run_live "$work/nosource")
grep -Fq 'no Apple Bluetooth firmware on this boot medium' <<<"$output"
! grep -Fq 'helper' "$work/nosource/log"
