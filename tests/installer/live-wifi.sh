#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
script="$repo_root/packaging/installer/runtime/kait2en-live-wifi"
helper="$repo_root/packaging/installer/runtime/install-wifi-firmware.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-live-wifi-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

chip=4364b3
board=trinidad
module=HRPN
vendor=u
version=7.7
prefix="brcmfmac${chip}-pcie"
identity="${board}-${module}-${vendor}-${version}"
# macOS names the blobs and the NVRAM after the board plus the antenna variant,
# but the .trx file after the bare board.
board_files="${board}-X3"
alloc_line="brcmfmac: brcmf_fw_alloc_request: using brcm/$prefix for chip BCM4364/4"

make_case() {
	local name=$1 log=$2
	local case_dir="$work/$name"

	mkdir -p "$case_dir/bin" "$case_dir/root" "$case_dir/source"
	printf firmware >"$case_dir/source/${board}.trx"
	printf clm >"$case_dir/source/${board_files}.clmb"
	printf txcap >"$case_dir/source/${board_files}.txcb"
	printf nvram \
		>"$case_dir/source/P-${board_files}_M-${module}_V-${vendor}__m-${version}.txt"

	printf '%s\n' '#!/bin/sh' "printf '%s\\n' \"\$*\" >>\"$case_dir/modprobe.log\"" \
		>"$case_dir/bin/modprobe"
	printf '%s\n' '#!/bin/sh' "cat \"$case_dir/kernel.log\"" \
		>"$case_dir/bin/dmesg"
	# The helper reads both sources, so the real boot log must stay out of the
	# test.
	printf '%s\n' '#!/bin/sh' 'exit 0' >"$case_dir/bin/journalctl"
	# Never touch the supplicant of the machine running the tests.
	printf '%s\n' '#!/bin/sh' "printf '%s\\n' \"\$*\" >>\"$case_dir/systemctl.log\"" \
		'exit 1' >"$case_dir/bin/systemctl"
	chmod 0755 "$case_dir/bin/modprobe" "$case_dir/bin/dmesg" \
		"$case_dir/bin/journalctl" "$case_dir/bin/systemctl"
	printf '%s' "$log" >"$case_dir/kernel.log"
	mkdir -p "$case_dir/net/wlan0/wireless"
}

run_case() {
	local name=$1
	local case_dir="$work/$name"

	PATH="$case_dir/bin:$PATH" \
		KAIT2EN_LIVE_FIRMWARE_SOURCE="$case_dir/source" \
		KAIT2EN_WIFI_HELPER="$helper" \
		KAIT2EN_LIVE_ROOT="$case_dir/root" \
		KAIT2EN_LIVE_NET_ROOT="$case_dir/net" \
		KAIT2EN_LIVE_REQUEST_TIMEOUT=0 \
		KAIT2EN_LIVE_LINK_TIMEOUT=0 \
		bash "$script"
}

expect_common_files() {
	local destination=$1 expected_bin=$2

	[[ -f "$destination/$expected_bin" ]]
	[[ -f "$destination/${prefix}.apple,${board_files}.clm_blob" ]]
	[[ -f "$destination/${prefix}.apple,${board_files}.txcap_blob" ]]
	# The antenna variant must never end up in front of the module details.
	[[ -f "$destination/${prefix}.apple,${identity}.txt" ]]
	[[ $(find "$destination" -maxdepth 1 -type f | wc -l) -eq 4 ]]
}

# Kernels that warn about every candidate they try: the most specific name in
# the log wins, including the antenna variant that cannot be derived otherwise.
make_case warned "$alloc_line
brcmfmac 0000:e5:00.0: Direct firmware load for brcm/${prefix}.apple,${identity}-X3.bin failed with error -2
brcmfmac 0000:e5:00.0: Direct firmware load for brcm/${prefix}.apple,${identity}.bin failed with error -2
brcmfmac 0000:e5:00.0: Direct firmware load for brcm/${prefix}.bin failed with error -2
"
run_case warned >"$work/warned.out"
expect_common_files "$work/warned/root/usr/lib/firmware/brcm" \
	"${prefix}.apple,${identity}-X3.bin"
grep -Fq 'Wi-Fi is ready in the live session: wlan0' "$work/warned.out"
# Loaded once up front, then reloaded so the driver picks up the new firmware.
[[ $(grep -c . "$work/warned/modprobe.log") -ge 3 ]]
grep -Fq -- '-r brcmfmac' "$work/warned/modprobe.log"

# Newer kernels try the board-specific candidates quietly and warn only about
# the last one. The board-specific name is then built from the NVRAM identity.
make_case quiet "$alloc_line
brcmfmac 0000:e5:00.0: Direct firmware load for brcm/${prefix}.bin failed with error -2
"
run_case quiet >"$work/quiet.out"
expect_common_files "$work/quiet/root/usr/lib/firmware/brcm" \
	"${prefix}.apple,${identity}.bin"

# Without a wireless chip the live session is left alone instead of failing the
# boot.
make_case absent ''
run_case absent >"$work/absent.out"
[[ ! -d "$work/absent/root/usr" ]]
grep -Fq 'leaving the live session unchanged' "$work/absent.out"

# A stick booted without the Apple firmware image must not be an error either.
make_case unequipped ''
rm -rf "$work/unequipped/source"
run_case unequipped >"$work/unequipped.out"
grep -Fq 'no Apple Wi-Fi firmware on this boot medium' "$work/unequipped.out"

# A packaging mistake has to be visible instead of silently skipped.
make_case broken ''
if PATH="$work/broken/bin:$PATH" \
	KAIT2EN_LIVE_FIRMWARE_SOURCE="$work/broken/source" \
	KAIT2EN_WIFI_HELPER="$work/broken/missing-helper" \
	KAIT2EN_LIVE_ROOT="$work/broken/root" \
	KAIT2EN_LIVE_REQUEST_TIMEOUT=0 \
	KAIT2EN_LIVE_LINK_TIMEOUT=0 \
	bash "$script" >/dev/null 2>&1; then
	printf 'a missing firmware helper was unexpectedly accepted\n' >&2
	exit 1
fi
