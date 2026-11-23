#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# Copyright (C) 2026 Th0masL <25387544+Th0masL@users.noreply.github.com>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd "$repo_root"

forbid_fq() {
	local needle=$1 file
	shift
	for file in "$@"; do
		if grep -Fq -- "$needle" "$file"; then
			printf 'Static check failed: "%s" must not appear in %s\n' "$needle" "$file" >&2
			exit 1
		fi
	done
}

expect_fq() {
	grep -Fq -- "$1" "$2" || {
		printf 'Static check failed: expected "%s" in %s\n' "$1" "$2" >&2
		exit 1
	}
}

shell_files=(
	auto-installer/build-in-container.sh
	auto-installer/build-input-kmod.sh
	auto-installer/initramfs/20-kait2en-input.sh.in
	auto-installer/initramfs/90-kait2en-updates.sh
	auto-installer/macos-release-bootstrap.sh.in
	auto-installer/runtime/install-bt-firmware.sh
	auto-installer/runtime/install-wifi-firmware.sh
	auto-installer/runtime/kait2en-install
	auto-installer/runtime/kait2en-launch-terminal
	auto-installer/runtime/kait2en-live-bluetooth
	auto-installer/runtime/kait2en-live-diagnostics
	auto-installer/runtime/kait2en-rescue
	auto-installer/runtime/kait2en-live-wifi
	auto-installer/runtime/kait2en-prepare
	apps/t2-touchbar/install.sh
	apps/t2-touchbar/uninstall.sh
	packaging/installer/runtime/kait2en-install
	scripts/fedora/build-installer.sh
	scripts/fedora/install-gdm-branding.sh
	scripts/fedora/install-dkms-modules.sh
	scripts/fedora/install-kernel-args.sh
	scripts/fedora/install-kernel-source.sh
	scripts/fedora/install-gpu-runtime-pm.sh
	scripts/fedora/install-i915.sh
	scripts/fedora/install-thunderbolt.sh
	scripts/fedora/install-wireless.sh
	scripts/fedora/install-rtc-sync.sh
	scripts/fedora/lib.sh
	scripts/macos/download-fedora-iso.sh
	scripts/macos/prepare-fedora-installer.sh
	scripts/copr/make-kernel-srpm.sh
	scripts/tests/edition-catalog.sh
	scripts/tests/iso-download.sh
	scripts/tests/installer-input.sh
	scripts/tests/install-launcher.sh
	scripts/tests/static-check.sh
	scripts/tests/prepare-install.sh
	scripts/tests/release-bootstrap.sh
	scripts/tests/run-step-continuation.sh
	scripts/tests/bt-firmware.sh
	scripts/tests/live-bluetooth.sh
	scripts/tests/rescue.sh
	scripts/tests/live-wifi.sh
	scripts/tests/transition-rpm.sh
	scripts/tests/terminal-launcher.sh
	scripts/tests/wifi-firmware.sh
)
for file in "${shell_files[@]}"; do
	bash -n "$file"
done

if command -v shellcheck >/dev/null 2>&1; then
	shellcheck --severity=warning -x "${shell_files[@]}"
fi

while IFS= read -r file; do
	python3 -c \
		'import ast, pathlib, sys; ast.parse(pathlib.Path(sys.argv[1]).read_text())' \
		"$file"
done < <(git ls-files 'auto-installer/anaconda-addon/*.py' \
	'auto-installer/anaconda-addon/**/*.py')

! grep -rInE --exclude-dir=target 'OEMDRV|rhdd3|inst\.dd=|inst\.ks=|kait2en\.wifi_required' \
	auto-installer/grub.cfg.in \
	auto-installer/initramfs \
	auto-installer/anaconda-addon \
	scripts/macos
! grep -rInE --exclude-dir=target 'brcmfmac(4364|4377).*alias|generic.*brcmfmac|brcmfmac[^ ]*-pcie\.txt' \
	auto-installer
# A KaiT2en entry without the input initramfs loses the keyboard it rescues.
awk '
$1 == "linux" && index($0, "${kait2en_common}") { entries++ }
$1 == "initrd" && $2 == "${kait2en_initrd}" { overlays++ }
END { exit !(entries >= 5 && entries == overlays) }
' auto-installer/grub.cfg.in
expect_fq 'plymouth.enable=0' auto-installer/grub.cfg.in
expect_fq 'nomodeset' auto-installer/grub.cfg.in
if grep -Eq '^set kait2en_blacklist=.*apple_gmux' auto-installer/grub.cfg.in; then
	exit 1
fi
! grep -InE 'INPUT_COMPAT_PATCH|compat_patch|auto-installer/patches' \
	auto-installer/runtime/kait2en-prepare
expect_fq '"$transition_source" "$target_kernel" "$work/rpm"' \
	auto-installer/runtime/kait2en-prepare
# The transition modules only live in the initramfs, so they must be forced in.
expect_fq 'dracut --force --force-drivers' \
	auto-installer/runtime/kait2en-prepare
forbid_fq '--add-drivers' auto-installer/runtime/kait2en-prepare

# DKMS drops every kernel's build before rebuilding any of them, so a kernel
# without headers has to be refused before anything is removed.
expect_fq 'require_kernel_headers' scripts/fedora/install-dkms-modules.sh
expect_fq 'require_kernel_headers()' scripts/fedora/lib.sh
expect_fq 't2-kernel-builder installation failed; continuing because it is optional' \
	scripts/fedora/install-apps.sh
expect_fq 'apps/t2-touchbar/install.sh' scripts/fedora/install-apps.sh
# The Touch Bar daemon is optional and asked for before any step runs.
expect_fq 'KAIT2EN_INSTALL_TOUCHBAR' scripts/fedora/install-apps.sh
expect_fq 'ask_touchbar' scripts/fedora/install.sh
# Only models with a Touch Bar are asked, and the first install must not
# switch the panel before the session can open it.
expect_fq 'has_touch_bar' scripts/fedora/install.sh
expect_fq 'has_touch_bar' scripts/fedora/install-apps.sh
# KaiT2en Vornehm is optional, offered only with GNOME and installed as
# the user who runs sudo.
expect_fq 'ask_vornehm' scripts/fedora/install.sh
expect_fq 'KAIT2EN_INSTALL_VORNEHM' scripts/fedora/install.sh
expect_fq 'as_vornehm_user bash "$SCRIPT_DIR/install-vornehm.sh"' scripts/fedora/install.sh
expect_fq 'systemctl disable kait2en-touchbar-attach.service' apps/t2-touchbar/install.sh
# ISOs up to v0.1.12 delegate to the old installer path.
expect_fq 'auto-installer/runtime' packaging/installer/runtime/kait2en-install
expect_fq '"$BIN" --detach' apps/t2-touchbar/uninstall.sh
expect_fq 'build_cargo_project "$APP_DIR"' apps/t2-touchbar/install.sh
expect_fq 'env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" make -C "$path" build' scripts/fedora/lib.sh
expect_fq 'device_group_is_live' apps/t2-touchbar/install.sh
expect_fq 'kait2en-touchbar --attach' \
	apps/t2-touchbar/integration/systemd/system/kait2en-touchbar-attach.service
expect_fq 't2-kernel-builder bundle is incomplete' \
	apps/t2-kernel-builder/install.sh
# A wrong origin URL must be repaired; dying leaves the user with no way out.
expect_fq 'repair_origin_url' auto-installer/runtime/kait2en-install
expect_fq 'repair_origin_url' auto-installer/runtime/kait2en-prepare
forbid_fq 'unexpected origin URL' \
	auto-installer/runtime/kait2en-install \
	auto-installer/runtime/kait2en-prepare
expect_fq '["dkms", "autoinstall", "-k", release]' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
expect_fq "DKMS_OVERRIDE.write_text('post_transaction=\"\"\\n'" \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
expect_fq 'rebuilt initramfs is missing required module' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
for module in acpi_tad applesmc macsmc_core macsmc_acpi macsmc_hwmon \
		macsmc_light macsmc_accel leds_macsmc macsmc_wdt rtc_macsmc \
		macsmc_power hid_apple hid_appletb_bl hid_appletb_kbd hid_magicmouse \
		appletbdrm apple_mfi_fastcharge apple_gmux apple-t2-bridge \
		apple-t2-vhci-hcd t2hid; do
	expect_fq "\"$module\"" apps/t2-kernel-builder/t2-kernel-builder-cleanup
done
expect_fq 'installation rolled back' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
expect_fq 'Path(f"/boot/initramfs-{release}.img").is_file()' \
	apps/t2-kernel-builder/t2-kernel-builder.py
forbid_fq '["dracut", "--force"' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
for symbol in MFD_MACSMC_CORE MACSMC_ACPI SENSORS_MACSMC_HWMON \
		MACSMC_LIGHT MACSMC_ACCEL LEDS_MACSMC MACSMC_WDT \
		RTC_DRV_MACSMC MACSMC_POWER; do
	expect_fq "$symbol" apps/t2-kernel-builder/engine/build.sh
done
expect_fq 'required T2 kernel module was rejected by Kconfig' \
	apps/t2-kernel-builder/engine/build.sh
expect_fq '--enable IIO' apps/t2-kernel-builder/engine/build.sh
expect_fq "grep -Eq '^\\+config[[:space:]]+MACSMC_ACPI" \
	apps/t2-kernel-builder/engine/build.sh
expect_fq 'if ((HAS_MACSMC_PATCHES)); then' \
	apps/t2-kernel-builder/engine/build.sh
expect_fq 'if ((HAS_AMD_DGPU)); then' \
	apps/t2-kernel-builder/engine/build.sh
expect_fq 'T2_REQUIRED_MODULES+=("${MACSMC_REQUIRED_MODULES[@]}")' \
	apps/t2-kernel-builder/engine/build.sh
expect_fq 'T2_REQUIRED_MODULES+=("${AMD_DGPU_REQUIRED_MODULES[@]}")' \
	apps/t2-kernel-builder/engine/build.sh
expect_fq '/etc/modprobe.d/kait2en-silent-blacklist.conf' \
	scripts/fedora/install-kernel-args.sh
expect_fq 'ln -sfn kait2en-multicall /usr/local/bin/edit-grub' scripts/fedora/install.sh
expect_fq 'ln -sfn kait2en-multicall /usr/local/bin/update-grub' scripts/fedora/install.sh
expect_fq 'ln -sfn kait2en-multicall /usr/local/bin/boot-macos' scripts/fedora/install.sh
expect_fq 'ln -sfn kait2en-multicall /usr/local/bin/update-modules' scripts/fedora/install.sh
expect_fq 'ln -sfn kait2en-multicall /usr/local/bin/update-apps' scripts/fedora/install.sh
expect_fq 'exec sudo grub2-mkconfig -o /boot/grub2/grub.cfg' \
	scripts/fedora/kait2en-multicall
expect_fq "printf 'install %s /bin/true" scripts/fedora/install-kernel-args.sh
expect_fq '"etc", "xdg", "autostart"' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
! grep -InE 'find_regular_user|home\.lstrip|os\.chown' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
expect_fq 'KAIT2EN_AUTOSTART_FILE:-/etc/xdg/autostart/kait2en-install.desktop' \
	auto-installer/runtime/kait2en-prepare
! grep -InE '\$HOME/\.config/autostart' \
	auto-installer/runtime/kait2en-install
if grep -rInE --exclude-dir=target 'kait2en-first-boot|KAIT2EN_FIRST_BOOT' \
		auto-installer; then
	exit 1
fi
# The live Wi-Fi helpers must ride along in the input initramfs and must stay
# inside /run, which never reaches the installed system.
expect_fq 'usr/lib/kait2en/kait2en-live-wifi' auto-installer/build-in-container.sh
expect_fq 'usr/lib/kait2en/kait2en-live-wifi.service' \
	auto-installer/build-in-container.sh
expect_fq 'usr/lib/kait2en/install-wifi-firmware.sh' \
	auto-installer/build-in-container.sh
expect_fq 'usr/lib/kait2en/kait2en-live-diagnostics' \
	auto-installer/build-in-container.sh
expect_fq 'runtime_units=/run/systemd/system' \
	auto-installer/initramfs/90-kait2en-updates.sh
expect_fq 'ExecStart=/run/kait2en/kait2en-live-wifi' \
	auto-installer/runtime/kait2en-live-wifi.service
! grep -rInE --exclude-dir=target 'kait2en-live-wifi' \
	auto-installer/anaconda-addon

expect_fq 'usr/lib/kait2en/kait2en-rescue' auto-installer/build-in-container.sh
expect_fq 'kait2en-rescue' auto-installer/initramfs/90-kait2en-updates.sh
expect_fq '/sysroot/usr/bin/kait2en-rescue' \
	auto-installer/initramfs/90-kait2en-updates.sh
expect_fq 'apfs' auto-installer/runtime/kait2en-rescue
expect_fq 'set-default-index' auto-installer/runtime/kait2en-rescue
forbid_fq '--set-default ' auto-installer/runtime/kait2en-rescue
! grep -rInE --exclude-dir=target 'kait2en-rescue' \
	auto-installer/anaconda-addon

# Bluetooth firmware is loaded from disk by BCM4377 alone. Every entry point has
# to check for that PCI function, and the UART .hcd path must stay out of here.
expect_fq '0x5fa0' auto-installer/runtime/install-bt-firmware.sh
expect_fq '0x5fa0' auto-installer/runtime/kait2en-live-bluetooth
expect_fq '0x5fa0' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
expect_fq 'BCM4377' scripts/macos/prepare-fedora-installer.sh
! grep -rInE --exclude-dir=target '\.hcd' auto-installer scripts/macos
expect_fq 'usr/lib/kait2en/install-bt-firmware.sh' \
	auto-installer/build-in-container.sh
expect_fq 'usr/lib/kait2en/kait2en-live-bluetooth' \
	auto-installer/build-in-container.sh
expect_fq 'usr/lib/kait2en/kait2en-live-bluetooth.service' \
	auto-installer/build-in-container.sh
expect_fq 'ExecStart=/run/kait2en/kait2en-live-bluetooth' \
	auto-installer/runtime/kait2en-live-bluetooth.service
! grep -rInE --exclude-dir=target 'kait2en-live-bluetooth' \
	auto-installer/anaconda-addon

# macOS Bash 3.2 treats an expanded empty array as unbound under `set -u`.
expect_fq 'ORIGINAL_ARGC=$#' scripts/macos/prepare-fedora-installer.sh
expect_fq 'if ((ORIGINAL_ARGC == 0)); then' scripts/macos/prepare-fedora-installer.sh
expect_fq 'plist_value "$disk" WholeDisk' scripts/macos/prepare-fedora-installer.sh
forbid_fq 'plist_value "$disk" Whole ' scripts/macos/prepare-fedora-installer.sh
expect_fq 'Select the orange EFI Boot entry for this USB drive.' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'The KaiT2en installation will continue automatically in a terminal.' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'Good: Secure Boot has been disabled.' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'Set Secure Boot to No Security.' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'Allow booting from external or removable media.' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'reconnect the USB drive and retry with --reuse-media' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'source "$SCRIPT_DIR/download-fedora-iso.sh"' \
	scripts/macos/prepare-fedora-installer.sh
expect_fq 'scripts/macos/download-fedora-iso.sh' \
	auto-installer/build-in-container.sh
expect_fq 'FEDORA_METALINK=' auto-installer/targets/fedora-45.conf
expect_fq 'FEDORA_ARCHIVE_BASEURL=' auto-installer/targets/fedora-45.conf
! grep -rIn 'FEDORA_BASEURL\|dl.fedoraproject.org/pub/fedora/linux/releases' \
	auto-installer/targets scripts/fedora auto-installer/build-in-container.sh
forbid_fq 'Keep no second driver disk connected' scripts/macos/prepare-fedora-installer.sh
forbid_fq 'before the intentional EFI customization' scripts/macos/prepare-fedora-installer.sh
expect_fq 'shasum -a 256 -c' auto-installer/macos-release-bootstrap.sh.in
expect_fq 'KAIT2EN_TTY:-/dev/tty' auto-installer/macos-release-bootstrap.sh.in
expect_fq 'uses: actions/upload-artifact@v6' .github/workflows/installer.yml
expect_fq 'uses: actions/download-artifact@v7' .github/workflows/installer.yml
! grep -InE 'uses: actions/(checkout|upload-artifact|download-artifact)@v4' \
	.github/workflows/installer.yml

bash scripts/tests/installer-input.sh

bash scripts/tests/wifi-firmware.sh
bash scripts/tests/bt-firmware.sh
bash scripts/tests/live-wifi.sh
bash scripts/tests/live-bluetooth.sh
bash scripts/tests/rescue.sh
bash scripts/tests/prepare-install.sh
bash scripts/tests/run-step-continuation.sh
bash scripts/tests/install-launcher.sh
bash scripts/tests/release-bootstrap.sh
bash scripts/tests/terminal-launcher.sh
bash scripts/tests/iso-download.sh
bash scripts/tests/edition-catalog.sh
bash scripts/tests/cargo-lock-alignment.sh
printf 'Installer static checks passed.\n'
