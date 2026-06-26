#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command grubby sed

REMOVE_ARGS=(
	intel_iommu
	iommu
	pm_async
	acpi_osi
	pcie_ports
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
	"acpi_osi=!Darwin"
	"'acpi_osi=Windows 2012'"
	"pcie_ports=native"
	"mem_sleep_default=deep"
	"t2gmux.force_igd=0"
	"initcall_blacklist=cmos_init,magicmouse_driver_init"
)

MODULE_BLACKLIST="module_blacklist=acpi_tad,applesmc,macsmc,hid_apple,hid_appletb_bl,hid_appletb_kbd,hid_magicmouse,appletbdrm,thunderbolt,apple_bce,apple_mfi_fastcharge,apple_gmux"

ADD_ARGS+=("$MODULE_BLACKLIST")
KERNEL_ARGS="${ADD_ARGS[*]}"

info "removing old or non-default kernel arguments"
for arg in "${REMOVE_ARGS[@]}"; do
	grubby --update-kernel=ALL --remove-args="$arg"
done

info "installing Kait2en kernel arguments and driver blacklist"
grubby --update-kernel=ALL --args="$KERNEL_ARGS"

info "current default kernel arguments:"
grubby --info=DEFAULT | sed -n 's/^args=//p'
