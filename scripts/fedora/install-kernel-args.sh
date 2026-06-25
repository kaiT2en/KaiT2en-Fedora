#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command grubby awk date install mktemp rm

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
	"initcall_blacklist=cmos_init,magicmouse_driver_init"
	"module_blacklist=acpi_tad,applesmc,macsmc,hid_apple,hid_appletb_bl,hid_appletb_kbd,hid_magicmouse,appletbdrm,thunderbolt,apple_bce,apple_mfi_fastcharge,apple_gmux"
)

model_specific_args() {
	local model
	[[ -r /sys/class/dmi/id/product_name ]] || return
	read -r model </sys/class/dmi/id/product_name

	case "$model" in
		MacBookPro15,1|MacBookPro15,3|MacBookPro16,1|MacBookPro16,4)
			ADD_ARGS+=("t2gmux.force_igd=1")
			info "enabling t2gmux.force_igd=1 for $model"
			;;
	esac
}

model_specific_args
KERNEL_ARGS="${ADD_ARGS[*]}"

repair_etc_default_grub() {
	local file=/etc/default/grub tmp backup
	[[ -f "$file" ]] || return

	backup="$file.kait2en.bak.$(date +%Y%m%d-%H%M%S)"
	tmp="$(mktemp)"
	install -o root -g root -m 0644 "$file" "$backup"

	awk -v args="$KERNEL_ARGS" '
		BEGIN { done = 0 }
		/^GRUB_CMDLINE_LINUX=/ {
			print "GRUB_CMDLINE_LINUX=\"" args "\""
			done = 1
			next
		}
		{ print }
		END {
			if (!done) {
				print "GRUB_CMDLINE_LINUX=\"" args "\""
			}
		}
	' "$file" >"$tmp"

	install -o root -g root -m 0644 "$tmp" "$file"
	rm -f "$tmp"
	info "backed up $file to $backup"
}

info "repairing /etc/default/grub kernel arguments"
repair_etc_default_grub

info "removing old or non-default kernel arguments"
for arg in "${REMOVE_ARGS[@]}"; do
	grubby --update-kernel=ALL --remove-args="$arg"
done

info "installing Kait2en kernel arguments and driver blacklist"
grubby --update-kernel=ALL --args="$KERNEL_ARGS"

info "current default kernel arguments:"
grubby --info=DEFAULT | sed -n 's/^args=//p'
