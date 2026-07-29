#!/usr/bin/env bash

# Install Apple's PCIe Bluetooth firmware for BCM4377 Macs. Only BCM4377
# (14e4:5fa0) uses this path; the older combo chips drive Bluetooth over UART and
# are not touched here.

set -Eeuo pipefail
shopt -s nullglob

source_dir=
root=/
pci_root=${KAIT2EN_PCI_ROOT:-/sys/bus/pci/devices}

usage() {
	printf 'Usage: %s --source DIR [--root DIR]\n' "${0##*/}"
}

die() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

note() {
	printf '%s\n' "$*"
}

while (($# > 0)); do
	case "$1" in
		--source)
			(($# >= 2)) || { usage >&2; exit 2; }
			source_dir=$2
			shift 2
			;;
		--root)
			(($# >= 2)) || { usage >&2; exit 2; }
			root=$2
			shift 2
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			usage >&2
			exit 2
			;;
	esac
done

[[ -d "$source_dir" ]] || die "Bluetooth firmware directory not found: $source_dir"
[[ -d "$root" ]] || die "target root not found: $root"

# 5fa0 is the only BCM4377 Bluetooth function. The driver also claims 5f69, 5f71
# and 5f72, but those chips sit in Apple Silicon Macs and never in a T2 Mac.
has_bcm4377() {
	local device vendor identifier

	for device in "$pci_root"/*; do
		[[ -r "$device/vendor" && -r "$device/device" ]] || continue
		vendor=$(<"$device/vendor")
		identifier=$(<"$device/device")
		[[ "$vendor" == 0x14e4 && "$identifier" == 0x5fa0 ]] || continue
		return 0
	done
	return 1
}

kernel_log() {
	journalctl -b -k --no-pager 2>/dev/null || :
	dmesg 2>/dev/null || :
}

# hci_bcm4377 asks for the firmware through firmware_request_nowarn(), so the
# firmware loader stays quiet and there is no "Direct firmware load ... failed"
# line like brcmfmac produces. The driver prints both names it tried itself.
requested_names() {
	kernel_log |
		sed -n "s|.*Unable to load firmware; tried '\(brcm/brcmbt[^']*\.bin\)' and '\(brcm/brcmbt[^']*\.bin\)'.*|\1 \2|p" |
		head -n 1
}

has_bcm4377 || die 'no BCM4377 Bluetooth function (14e4:5fa0) in this Mac'

# The board type comes from a DMI table inside the driver. Without a match the
# probe is aborted before any firmware is requested, and no file can fix that.
if kernel_log | grep -Fq 'unable to determine board type'; then
	printf 'Error: this Mac is missing from the kernel Bluetooth board table.\n' >&2
	printf '  model: %s\n' \
		"$(cat /sys/class/dmi/id/product_name 2>/dev/null || printf unknown)" >&2
	printf '  hci_bcm4377 stops before it asks for firmware, so installing it\n' >&2
	printf '  cannot help. Please report this model.\n' >&2
	exit 1
fi

# Apple names the blobs after chip, stepping, board and the antenna vendor, for
# example BCM4377B3_x.y_PCIE_formosa_MFG_USI_..._PROD.signed.bin. The driver
# builds its request from the same four parts, so they can be mapped directly.
name_re='^BCM([0-9]{4})([A-Z][0-9])_.*_PCIE_([^_]+)_MFG_([^_]+)_.*_PROD\.signed\.bin$'

candidate_base=
candidate_bin=
candidate_ptb=

describe_candidate() {
	local file=$1 name chip stepping board vendor suffix ptb_files
	name=${file##*/}
	[[ "$name" =~ $name_re ]] || return 1

	chip=${BASH_REMATCH[1]}
	stepping=${BASH_REMATCH[2]}
	board=${BASH_REMATCH[3]}
	vendor=${BASH_REMATCH[4]}
	[[ "$chip" == 4377 ]] || return 1

	case "$vendor" in
		GEN) suffix= ;;
		MUR) suffix=-m ;;
		USI) suffix=-u ;;
		*) return 1 ;;
	esac

	ptb_files=("$source_dir/BCM${chip}${stepping}__PCIE_${board}_MFG_${vendor}__PRODK_R_.ptb")
	((${#ptb_files[@]} == 1)) || return 1

	candidate_base="brcmbt${chip}${stepping,,}-apple,${board,,}${suffix}"
	candidate_bin=$file
	candidate_ptb=${ptb_files[0]}
}

bases=()
bins=()
ptbs=()
for blob in "$source_dir"/BCM4377*_PROD.signed.bin; do
	describe_candidate "$blob" || continue
	bases+=("$candidate_base")
	bins+=("$candidate_bin")
	ptbs+=("$candidate_ptb")
done
((${#bases[@]} > 0)) ||
	die "no usable BCM4377 production firmware and PTB pair in $source_dir"

if [[ -n ${KAIT2EN_BT_FIRMWARE_REQUEST:-} ]]; then
	read -r -a requested <<<"$KAIT2EN_BT_FIRMWARE_REQUEST"
else
	read -r -a requested <<<"$(requested_names)"
fi

requested_bases=()
for request in "${requested[@]}"; do
	[[ "$request" == brcm/brcmbt4377*.bin ]] ||
		die "unexpected Bluetooth firmware request: $request"
	request=${request#brcm/}
	requested_bases+=("${request%.bin}")
done

selected=-1
if ((${#requested_bases[@]} > 0)); then
	for index in "${!bases[@]}"; do
		for base in "${requested_bases[@]}"; do
			[[ "${bases[index]}" == "$base" ]] || continue
			selected=$index
			break 2
		done
	done
	if ((selected < 0)); then
		# The filename records what Apple built, the request records what the chip
		# reports. They can only disagree about board and antenna vendor, and only
		# the requested name can ever be loaded, so install under it as long as
		# the chip and stepping match and there is nothing to choose from.
		((${#bases[@]} == 1)) || die \
			"none of the Bluetooth blobs in $source_dir matches ${requested_bases[*]}"
		[[ "${bases[0]%%-apple,*}" == "${requested_bases[0]%%-apple,*}" ]] || die \
			"the Bluetooth firmware in $source_dir is for another chip than ${requested_bases[0]}"
		note "Warning: ${bins[0]##*/} is not named like ${requested_bases[0]},"
		note '  installing it under the requested name anyway.'
		bases[0]=${requested_bases[0]}
		selected=0
	fi
	note "hci_bcm4377 asked for: ${requested_bases[*]}"
else
	((${#bases[@]} == 1)) || die \
		"hci_bcm4377 did not report a firmware request and $source_dir holds ${#bases[@]} candidates"
	selected=0
	note 'hci_bcm4377 reported no firmware request, using the Apple filename'
fi

target_base=${bases[selected]}
dest_dir="$root/usr/lib/firmware/brcm"

install -d -m 0755 "$dest_dir"
install -m 0644 "${bins[selected]}" "$dest_dir/$target_base.bin"
install -m 0644 "${ptbs[selected]}" "$dest_dir/$target_base.ptb"

printf 'KaiT2en Bluetooth firmware installed:\n'
printf '  /usr/lib/firmware/brcm/%s\n' "$target_base.bin" "$target_base.ptb"
