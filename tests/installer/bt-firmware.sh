#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
helper="$repo_root/packaging/installer/runtime/install-bt-firmware.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-bt-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

# hci_bcm4377 asks for brcm/brcmbt<chip><stepping>-<board_type>-<vendor>.bin and
# falls back to the name without the vendor. Apple names the same firmware after
# chip, stepping, board and the antenna vendor of this Mac.
apple_bin() {
	printf 'BCM4377%s_18.6.116.0.11_PCIE_%s_MFG_%s_ANT_A_FA0_PROD.signed.bin\n' \
		"$1" "$2" "$3"
}

apple_ptb() {
	printf 'BCM4377%s__PCIE_%s_MFG_%s__PRODK_R_.ptb\n' "$1" "$2" "$3"
}

# The helper only ever touches Macs with the BCM4377 Bluetooth function.
make_pci_root() {
	local root=$1 identifier=${2:-0x5fa0}
	mkdir -p "$root/0000:03:00.0"
	printf '0x14e4\n' >"$root/0000:03:00.0/vendor"
	printf '%s\n' "$identifier" >"$root/0000:03:00.0/device"
}

make_log_stubs() {
	local bin_dir=$1
	shift
	mkdir -p "$bin_dir"
	printf '%s\n' '#!/bin/sh' 'exit 0' >"$bin_dir/journalctl"
	{
		printf '%s\n' '#!/bin/sh'
		if (($# > 0)); then
			printf 'echo "%s"\n' "$@"
		fi
	} >"$bin_dir/dmesg"
	chmod 0755 "$bin_dir/journalctl" "$bin_dir/dmesg"
}

unable_line() {
	printf "hci_bcm4377 0000:03:00.0: Unable to load firmware; tried '%s' and '%s'" \
		"$1" "$2"
}

# The straightforward case: one Apple blob, and the driver names both candidates
# it tried. Repeated for every antenna vendor Apple uses.
run_case() {
	local name=$1 stepping=$2 board=$3 vendor=$4 suffix=$5
	local case_dir="$work/$name"
	local source="$case_dir/source" root="$case_dir/root" bin_dir="$case_dir/bin"
	local base="brcmbt4377${stepping,,}-apple,${board,,}"

	mkdir -p "$source" "$root"
	printf firmware >"$source/$(apple_bin "$stepping" "$board" "$vendor")"
	printf ptb >"$source/$(apple_ptb "$stepping" "$board" "$vendor")"
	make_pci_root "$case_dir/pci"
	make_log_stubs "$bin_dir" "$(unable_line "brcm/$base$suffix.bin" "brcm/$base.bin")"

	PATH="$bin_dir:$PATH" KAIT2EN_PCI_ROOT="$case_dir/pci" \
		bash "$helper" --source "$source" --root "$root" >/dev/null

	[[ -f "$root/usr/lib/firmware/brcm/$base$suffix.bin" ]]
	[[ -f "$root/usr/lib/firmware/brcm/$base$suffix.ptb" ]]
	[[ $(find "$root/usr/lib/firmware/brcm" -maxdepth 1 -type f | wc -l) -eq 2 ]]
}

run_case usi B3 formosa USI -u
run_case murata B3 formosa MUR -m
# A board without an antenna vendor in OTP is why the driver has a second,
# shorter candidate name at all.
run_case generic B3 formosa GEN ''

# Apple ships more than one antenna vendor on some Macs. Only the chip knows
# which one it is, so the requested name decides.
several="$work/several"
mkdir -p "$several/source" "$several/root"
for pair in USI:-u MUR:-m; do
	printf firmware >"$several/source/$(apple_bin B3 formosa "${pair%%:*}")"
	printf ptb >"$several/source/$(apple_ptb B3 formosa "${pair%%:*}")"
done
make_pci_root "$several/pci"
make_log_stubs "$several/bin" \
	"$(unable_line 'brcm/brcmbt4377b3-apple,formosa-m.bin' \
		'brcm/brcmbt4377b3-apple,formosa.bin')"
PATH="$several/bin:$PATH" KAIT2EN_PCI_ROOT="$several/pci" \
	bash "$helper" --source "$several/source" --root "$several/root" >/dev/null
[[ -f "$several/root/usr/lib/firmware/brcm/brcmbt4377b3-apple,formosa-m.bin" ]]
[[ ! -e "$several/root/usr/lib/firmware/brcm/brcmbt4377b3-apple,formosa-u.bin" ]]

# Without a request in the log the Apple filename is the only source, which is
# good enough as long as it is unambiguous.
quiet="$work/quiet"
mkdir -p "$quiet/source" "$quiet/root"
printf firmware >"$quiet/source/$(apple_bin B3 formosa USI)"
printf ptb >"$quiet/source/$(apple_ptb B3 formosa USI)"
make_pci_root "$quiet/pci"
make_log_stubs "$quiet/bin"
PATH="$quiet/bin:$PATH" KAIT2EN_PCI_ROOT="$quiet/pci" \
	bash "$helper" --source "$quiet/source" --root "$quiet/root" >/dev/null
[[ -f "$quiet/root/usr/lib/firmware/brcm/brcmbt4377b3-apple,formosa-u.bin" ]]

# The chip reads its antenna vendor from OTP, the filename only records what
# Apple built. If they disagree and there is nothing to choose from, the name the
# driver asks for is the only one that can ever be loaded.
mismatch="$work/mismatch"
mkdir -p "$mismatch/source" "$mismatch/root"
printf firmware >"$mismatch/source/$(apple_bin B3 formosa USI)"
printf ptb >"$mismatch/source/$(apple_ptb B3 formosa USI)"
make_pci_root "$mismatch/pci"
make_log_stubs "$mismatch/bin" \
	"$(unable_line 'brcm/brcmbt4377b3-apple,formosa-m.bin' \
		'brcm/brcmbt4377b3-apple,formosa.bin')"
mismatch_output=$(
	PATH="$mismatch/bin:$PATH" KAIT2EN_PCI_ROOT="$mismatch/pci" \
		bash "$helper" --source "$mismatch/source" --root "$mismatch/root"
)
[[ -f "$mismatch/root/usr/lib/firmware/brcm/brcmbt4377b3-apple,formosa-m.bin" ]]
[[ -f "$mismatch/root/usr/lib/firmware/brcm/brcmbt4377b3-apple,formosa-m.ptb" ]]
grep -Fq 'Warning:' <<<"$mismatch_output"

fails() {
	local label=$1
	shift
	if "$@" >/dev/null 2>&1; then
		printf '%s was unexpectedly accepted\n' "$label" >&2
		exit 1
	fi
}

# Two candidates and nothing to choose from: guessing here would install a file
# the chip never asks for.
ambiguous="$work/ambiguous"
mkdir -p "$ambiguous/source" "$ambiguous/root"
for vendor in USI MUR; do
	printf firmware >"$ambiguous/source/$(apple_bin B3 formosa "$vendor")"
	printf ptb >"$ambiguous/source/$(apple_ptb B3 formosa "$vendor")"
done
make_pci_root "$ambiguous/pci"
make_log_stubs "$ambiguous/bin"
fails 'an ambiguous set of Bluetooth blobs' \
	env PATH="$ambiguous/bin:$PATH" KAIT2EN_PCI_ROOT="$ambiguous/pci" \
	bash "$helper" --source "$ambiguous/source" --root "$ambiguous/root"
[[ ! -d "$ambiguous/root/usr/lib/firmware" ]]

# A blob without its PTB counterpart cannot start the controller.
lonely="$work/lonely"
mkdir -p "$lonely/source" "$lonely/root"
printf firmware >"$lonely/source/$(apple_bin B3 formosa USI)"
make_pci_root "$lonely/pci"
make_log_stubs "$lonely/bin"
fails 'a firmware blob without its PTB' \
	env PATH="$lonely/bin:$PATH" KAIT2EN_PCI_ROOT="$lonely/pci" \
	bash "$helper" --source "$lonely/source" --root "$lonely/root"

# Without a board type the driver stops before it asks for firmware, so no file
# can help and installing one would only hide the real cause.
unknown="$work/unknown"
mkdir -p "$unknown/source" "$unknown/root"
printf firmware >"$unknown/source/$(apple_bin B3 formosa USI)"
printf ptb >"$unknown/source/$(apple_ptb B3 formosa USI)"
make_pci_root "$unknown/pci"
make_log_stubs "$unknown/bin" 'hci_bcm4377 0000:03:00.0: unable to determine board type'
fails 'a Mac without a known Bluetooth board type' \
	env PATH="$unknown/bin:$PATH" KAIT2EN_PCI_ROOT="$unknown/pci" \
	bash "$helper" --source "$unknown/source" --root "$unknown/root"
[[ ! -d "$unknown/root/usr/lib/firmware" ]]

# Macs with the older combo chip drive Bluetooth over UART and must be left
# alone, even when Bluetooth firmware happens to sit on the stick.
other="$work/other"
mkdir -p "$other/source" "$other/root"
printf firmware >"$other/source/$(apple_bin B3 formosa USI)"
printf ptb >"$other/source/$(apple_ptb B3 formosa USI)"
make_pci_root "$other/pci" 0x4464
make_log_stubs "$other/bin"
fails 'a Mac without the BCM4377 Bluetooth function' \
	env PATH="$other/bin:$PATH" KAIT2EN_PCI_ROOT="$other/pci" \
	bash "$helper" --source "$other/source" --root "$other/root"
[[ ! -d "$other/root/usr/lib/firmware" ]]

# Firmware from a different chip is never installed under the requested name.
foreign="$work/foreign"
mkdir -p "$foreign/source" "$foreign/root"
printf firmware >"$foreign/source/$(apple_bin B3 formosa USI)"
printf ptb >"$foreign/source/$(apple_ptb B3 formosa USI)"
make_pci_root "$foreign/pci"
make_log_stubs "$foreign/bin" \
	"$(unable_line 'brcm/brcmbt4377b2-apple,tahiti-u.bin' \
		'brcm/brcmbt4377b2-apple,tahiti.bin')"
fails 'firmware for another chip stepping' \
	env PATH="$foreign/bin:$PATH" KAIT2EN_PCI_ROOT="$foreign/pci" \
	bash "$helper" --source "$foreign/source" --root "$foreign/root"
[[ ! -d "$foreign/root/usr/lib/firmware" ]]
