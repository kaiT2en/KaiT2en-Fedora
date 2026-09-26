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

REMOVE_ARGS=(
	"'acpi_osi=Windows 2012'"
	"acpi_osi='Windows 2012'"
	acpi_osi
	intel_iommu
	iommu
	pm_async
	brcmfmac.p2pon
	pci
	pcie_ports
	pcie_aspm
	pcie_aspm.policy
	nvme_core.default_ps_max_latency_us
	apple_gmux.force_igd
	t2gmux.force_igd
	i915.enable_guc
	mem_sleep_default
	initcall_blacklist
	module_blacklist
)

ADD_ARGS=(
	"amdgpu.aspm"
	"i915.enable_guc=2"
	"intel_iommu=on"
	"iommu=pt"
	"pm_async=off"
	"brcmfmac.p2pon=0"
	"pcie_aspm=force"
	"pcie_aspm.policy=powersave"
	"pcie_ports=compat"
	"mem_sleep_default=deep"
)

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
