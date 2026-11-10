#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command chmod grubby grep install sed

repair_empty_grub_cmdline() {
	local arg cmdline escaped
	local -a args kept_args=()

	[[ -f /etc/default/grub ]] || return 0
	grep -qx 'GRUB_CMDLINE_LINUX=""' /etc/default/grub || return 0
	[[ -r /etc/kernel/cmdline ]] ||
		fail "cannot repair empty GRUB_CMDLINE_LINUX without /etc/kernel/cmdline"

	IFS= read -r cmdline </etc/kernel/cmdline
	read -r -a args <<<"$cmdline"
	for arg in "${args[@]}"; do
		case "$arg" in
			BOOT_IMAGE=*|root=*|rootflags=*|ro)
				continue
				;;
		esac
		kept_args+=("$arg")
	done

	((${#kept_args[@]})) ||
		fail "cannot repair GRUB_CMDLINE_LINUX from an empty kernel command line"

	cmdline="${kept_args[*]}"
	escaped="${cmdline//\\/\\\\}"
	escaped="${escaped//&/\\&}"
	escaped="${escaped//|/\\|}"
	sed -i \
		"s|^GRUB_CMDLINE_LINUX=\"\"$|GRUB_CMDLINE_LINUX=\"$escaped\"|" \
		/etc/default/grub
	grep -q '^GRUB_CMDLINE_LINUX="[^"]' /etc/default/grub ||
		fail "failed to repair GRUB_CMDLINE_LINUX"
	info "restored GRUB_CMDLINE_LINUX from the installed kernel command line"
}

# Arguments older installs set that current ones leave out. With the runtime
# wireless patches the BCM4377 and BCM4364 pass S3 with asynchronous suspend
# and resume.
REMOVE_ARGS=(
	"pm_async=off"
	"brcmfmac.p2pon=0"
)

ADD_ARGS=(
	"amdgpu.aspm=1"
	"i915.enable_guc=2"
	"intel_iommu=on"
	"iommu=pt"
	"pcie_aspm=force"
	"pcie_aspm.policy=powersave"
	"pcie_ports=compat"
	"pci=noaer"
	"mem_sleep_default=deep"
)

# The Ethernet controllers in iMacs and the iMac Pro do not survive forced
# ASPM, so leave ASPM to the firmware there.
model=""
[[ -r /sys/class/dmi/id/product_name ]] &&
	read -r model </sys/class/dmi/id/product_name
if [[ "$model" == iMac* ]]; then
	kept_args=()
	for arg in "${ADD_ARGS[@]}"; do
		[[ "$arg" == pcie_aspm=force ]] || kept_args+=("$arg")
	done
	ADD_ARGS=("${kept_args[@]}")
	info "not forcing PCIe ASPM on $model"
fi

has_titan_ridge() {
	local dev vendor device

	for dev in /sys/bus/pci/devices/*; do
		read -r vendor <"$dev/vendor" || continue
		read -r device <"$dev/device" || continue
		[[ $vendor == 0x8086 && ( $device == 0x15e8 || $device == 0x15eb ) ]] &&
			return 0
	done
	return 1
}
if has_titan_ridge; then
	kept_args=()
	for arg in "${ADD_ARGS[@]}"; do
		[[ "$arg" == pcie_ports=compat ]] || kept_args+=("$arg")
	done
	ADD_ARGS=("${kept_args[@]}")
	REMOVE_ARGS+=("pcie_ports=compat")
	info "leaving the PCIe port services on for the Titan Ridge controller"
fi

INITCALL_BLACKLIST="initcall_blacklist=cmos_init,magicmouse_driver_init"
BLACKLIST_MODULES=(
	acpi_tad
	applesmc
	macsmc
	hid_apple
	hid_appletb_bl
	hid_appletb_kbd
	hid_magicmouse
	appletbdrm
	apple_bce
	apple_mfi_fastcharge
	apple_gmux
	sbs
)
MODULE_BLACKLIST="module_blacklist=$(IFS=,; printf '%s' "${BLACKLIST_MODULES[*]}")"
SILENT_BLACKLIST_CONF="/etc/modprobe.d/kait2en-silent-blacklist.conf"

ADD_ARGS+=("$INITCALL_BLACKLIST" "$MODULE_BLACKLIST")

KERNEL_ARGS="${ADD_ARGS[*]}"
OLD_KERNEL_ARGS="${REMOVE_ARGS[*]}"

info "updating Kait2en kernel arguments and driver blacklist"
grubby --update-kernel=ALL \
	--remove-args="$OLD_KERNEL_ARGS" \
	--args="$KERNEL_ARGS"
repair_empty_grub_cmdline

install -d -m 0755 "${SILENT_BLACKLIST_CONF%/*}"
{
	printf '# Managed by scripts/fedora/install-kernel-args.sh\n'
	for module in "${BLACKLIST_MODULES[@]}"; do
		printf 'install %s /bin/true\n' "$module"
	done
} >"$SILENT_BLACKLIST_CONF"
chmod 0644 "$SILENT_BLACKLIST_CONF"

info "current default kernel arguments:"
grubby --info=DEFAULT | sed -n 's/^args=//p'
