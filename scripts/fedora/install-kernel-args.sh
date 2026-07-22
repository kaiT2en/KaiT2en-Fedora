#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command grubby sed

has_intel_pci_device() {
	local device path wanted

	for path in /sys/bus/pci/devices/*; do
		[[ -r "$path/vendor" && -r "$path/device" ]] || continue
		[[ $(<"$path/vendor") == 0x8086 ]] || continue
		device="$(<"$path/device")"
		for wanted in "$@"; do
			[[ "$device" == "$wanted" ]] && return 0
		done
	done

	return 1
}

# Clean up both quoted forms previously installed by KaiT2en.
LEGACY_ARGS=(
	"'acpi_osi=Windows 2012'"
	"acpi_osi='Windows 2012'"
)

REMOVE_ARGS=(
	intel_iommu
	iommu
	pm_async
	pcie_aspm
	pcie_aspm.policy
	nvme_core.default_ps_max_latency_us
	apple_gmux.force_igd
	t2gmux.force_igd
	mem_sleep_default
	initcall_blacklist
	module_blacklist
)

ADD_ARGS=(
	"intel_iommu=on"
	"iommu=pt"
	"pm_async=off"
	"mem_sleep_default=deep"
	"initcall_blacklist=cmos_init,magicmouse_driver_init"
)

MODULE_BLACKLIST="module_blacklist=acpi_tad,applesmc,macsmc,hid_apple,hid_appletb_bl,hid_appletb_kbd,hid_magicmouse,appletbdrm,apple_bce,apple_mfi_fastcharge,apple_gmux"

ADD_ARGS+=("$MODULE_BLACKLIST")

if has_intel_pci_device 0x15e8 0x15eb; then
	info "Titan Ridge detected; removing obsolete ACPI OSI and PCIe port overrides"
	grubby --update-kernel=ALL --remove-args="acpi_osi"
	grubby --update-kernel=ALL --remove-args="pcie_ports"
elif has_intel_pci_device 0x8a0d 0x8a17; then
	info "Ice Lake Thunderbolt detected; installing the required non-Darwin ACPI path"
	grubby --update-kernel=ALL --remove-args="acpi_osi"
	ADD_ARGS+=("acpi_osi=!Darwin")
else
	warn "unknown Thunderbolt generation; leaving ACPI OSI and PCIe port arguments unchanged"
fi

KERNEL_ARGS="${ADD_ARGS[*]}"

info "removing old or non-default kernel arguments"
for arg in "${LEGACY_ARGS[@]}"; do
	grubby --update-kernel=ALL --remove-args="$arg"
done
for arg in "${REMOVE_ARGS[@]}"; do
	grubby --update-kernel=ALL --remove-args="$arg"
done

info "installing Kait2en kernel arguments and driver blacklist"
grubby --update-kernel=ALL --args="$KERNEL_ARGS"

info "current default kernel arguments:"
grubby --info=DEFAULT | sed -n 's/^args=//p'
