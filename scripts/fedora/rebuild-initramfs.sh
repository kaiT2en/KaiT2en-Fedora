#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dracut install

# t2smc is included so its SMC-backed RTC registers during the initramfs and
# the kernel sets the wall clock early, instead of falling back to epoch until
# the driver loads from the real root.
# t2sbs follows t2smc so BAT0 already carries charge_control_end_threshold
# when udev and UPower in the real root first see it. UPower decides only then
# whether the battery supports a charge limit.
EARLY_MODULES=(apple-t2-bridge apple-t2-vhci-hcd t2hid t2smc t2sbs)
DRACUT_CONF="/etc/dracut.conf.d/90-kait2en-input.conf"
KVER="$(kernel_release)"
INITRAMFS="/boot/initramfs-$KVER.img"

install -d -m 0755 /etc/dracut.conf.d
printf '# Managed by scripts/fedora/rebuild-initramfs.sh\nforce_drivers+=" %s "\n' \
	"${EARLY_MODULES[*]}" >"$DRACUT_CONF"

info "rebuilding initramfs for $KVER"
dracut --force "$INITRAMFS" "$KVER"

info "initramfs rebuilt"
