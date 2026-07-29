#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
helper="$repo_root/packaging/installer/runtime/install-wifi-firmware.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-wifi-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

# macOS names the .clmb, .txcb and NVRAM files after the board plus the antenna
# variant of the Mac, and the .trx file after the bare board. brcmfmac asks
# either for <board>-<module>-<vendor>-<version> or for <board>-<antenna>, so
# the antenna variant must never be pasted in front of the module details.
run_case() {
	local name=$1 chip=$2 base=$3 antenna=$4 module=$5 vendor=$6 version=$7
	local board="${base}${antenna:+-$antenna}"
	local source="$work/$name/source" root="$work/$name/root"
	local prefix="brcmfmac${chip}-pcie"
	local request="brcm/${prefix}.apple,${base}-${module}-${vendor}-${version}${antenna:+-$antenna}.bin"

	mkdir -p "$source" "$root"
	printf firmware >"$source/${base}.trx"
	printf clm >"$source/${board}.clmb"
	printf txcap >"$source/${board}.txcb"
	printf nvram >"$source/P-${board}_M-${module}_V-${vendor}__m-${version}.txt"

	KAIT2EN_FIRMWARE_REQUEST=$request bash "$helper" --source "$source" --root "$root" >/dev/null

	local destination="$root/usr/lib/firmware/brcm"
	[[ -f "$destination/${request#brcm/}" ]]
	[[ -f "$destination/${prefix}.apple,${board}.clm_blob" ]]
	[[ -f "$destination/${prefix}.apple,${board}.txcap_blob" ]]
	[[ -f "$destination/${prefix}.apple,${base}-${module}-${vendor}-${version}.txt" ]]
	[[ $(find "$destination" -maxdepth 1 -type f | wc -l) -eq 4 ]]
}

# All three are the files macOS actually carries on those Macs.
run_case bcm4364b3 4364b3 trinidad X3 HRPN u 7.7
run_case bcm4364b2 4364b2 kauai X3 HRPN u 7.5
run_case bcm4377 4377b3 tahiti X3 SPPR m 3.1
# No Mac without an antenna variant has turned up yet, so this one is made up.
run_case noantenna 4364b3 sample '' HRPN u 7.5

# Newer kernels try the board-specific candidates quietly and warn only about
# the last one. The helper then names the file from the chip prefix the driver
# reports plus the identity of the NVRAM file.
detect="$work/detect"
mkdir -p "$detect/bin" "$detect/source" "$detect/root"
printf firmware >"$detect/source/trinidad.trx"
printf clm >"$detect/source/trinidad-X3.clmb"
printf txcap >"$detect/source/trinidad-X3.txcb"
printf nvram >"$detect/source/P-trinidad-X3_M-HRPN_V-u__m-7.7.txt"
printf '%s\n' '#!/bin/sh' 'exit 0' >"$detect/bin/journalctl"
printf '%s\n' '#!/bin/sh' \
	'echo "brcmfmac: brcmf_fw_alloc_request: using brcm/brcmfmac4364b3-pcie for chip BCM4364/4"' \
	>"$detect/bin/dmesg"
chmod 0755 "$detect/bin/journalctl" "$detect/bin/dmesg"
PATH="$detect/bin:$PATH" bash "$helper" \
	--source "$detect/source" --root "$detect/root" >/dev/null
[[ -f "$detect/root/usr/lib/firmware/brcm/brcmfmac4364b3-pcie.apple,trinidad-HRPN-u-7.7.bin" ]]
[[ -f "$detect/root/usr/lib/firmware/brcm/brcmfmac4364b3-pcie.apple,trinidad-X3.clm_blob" ]]
[[ -f "$detect/root/usr/lib/firmware/brcm/brcmfmac4364b3-pcie.apple,trinidad-HRPN-u-7.7.txt" ]]

# Kernels that warn about every candidate name the most specific one first, so
# the helper takes it verbatim. This is the path the Fedora live kernel uses,
# and the one that shipped a dead NVRAM name before.
warned="$work/warned"
mkdir -p "$warned/bin" "$warned/source" "$warned/root"
printf firmware >"$warned/source/trinidad.trx"
printf clm >"$warned/source/trinidad-X3.clmb"
printf txcap >"$warned/source/trinidad-X3.txcb"
printf nvram >"$warned/source/P-trinidad-X3_M-HRPN_V-u__m-7.7.txt"
printf '%s\n' '#!/bin/sh' 'exit 0' >"$warned/bin/journalctl"
{
	printf '%s\n' '#!/bin/sh'
	printf 'echo "%s"\n' \
		'brcmfmac: brcmf_fw_alloc_request: using brcm/brcmfmac4364b3-pcie for chip BCM4364/4' \
		'brcmfmac 0000:e5:00.0: Direct firmware load for brcm/brcmfmac4364b3-pcie.apple,trinidad-HRPN-u-7.7-X3.bin failed with error -2' \
		'brcmfmac 0000:e5:00.0: Direct firmware load for brcm/brcmfmac4364b3-pcie.apple,trinidad-X3.bin failed with error -2' \
		'brcmfmac 0000:e5:00.0: Direct firmware load for brcm/brcmfmac4364b3-pcie.bin failed with error -2'
} >"$warned/bin/dmesg"
chmod 0755 "$warned/bin/journalctl" "$warned/bin/dmesg"
PATH="$warned/bin:$PATH" bash "$helper" \
	--source "$warned/source" --root "$warned/root" >/dev/null
warned_dest="$warned/root/usr/lib/firmware/brcm"
[[ -f "$warned_dest/brcmfmac4364b3-pcie.apple,trinidad-HRPN-u-7.7-X3.bin" ]]
[[ -f "$warned_dest/brcmfmac4364b3-pcie.apple,trinidad-X3.clm_blob" ]]
[[ -f "$warned_dest/brcmfmac4364b3-pcie.apple,trinidad-X3.txcap_blob" ]]
[[ -f "$warned_dest/brcmfmac4364b3-pcie.apple,trinidad-HRPN-u-7.7.txt" ]]

# A generic fallback name is intentionally rejected. The helper must use the
# hardware-specific filename emitted by brcmfmac, exactly like the manual guide.
mkdir -p "$work/reject/source" "$work/reject/root"
printf x >"$work/reject/source/board.trx"
printf x >"$work/reject/source/board.clmb"
printf x >"$work/reject/source/board.txcb"
printf x >"$work/reject/source/P-board_M-module_V-vendor__m-1.txt"
if KAIT2EN_FIRMWARE_REQUEST=brcm/brcmfmac4364-pcie.txt \
	bash "$helper" --source "$work/reject/source" --root "$work/reject/root" \
	>/dev/null 2>&1; then
	printf 'generic brcmfmac firmware request was unexpectedly accepted\n' >&2
	exit 1
fi
