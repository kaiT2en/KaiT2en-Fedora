#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd "$repo_root"

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
	scripts/fedora/build-installer.sh
	scripts/fedora/install-gdm-branding.sh
	scripts/fedora/install-dkms-modules.sh
	scripts/fedora/install-kernel-args.sh
	scripts/fedora/lib.sh
	scripts/macos/download-fedora-iso.sh
	scripts/macos/prepare-fedora-installer.sh
	scripts/copr/make-kernel-srpm.sh
	scripts/tests/edition-catalog.sh
	scripts/tests/iso-download.sh
	scripts/tests/install-launcher.sh
	scripts/tests/static-check.sh
	scripts/tests/prepare-install.sh
	scripts/tests/release-bootstrap.sh
	scripts/tests/bt-firmware.sh
	scripts/tests/live-bluetooth.sh
	scripts/tests/rescue.sh
	scripts/tests/live-wifi.sh
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
grep -Fq 'plymouth.enable=0' auto-installer/grub.cfg.in
grep -Fq 'nomodeset' auto-installer/grub.cfg.in
if grep -Eq '^set kait2en_blacklist=.*apple_gmux' auto-installer/grub.cfg.in; then
	exit 1
fi
! grep -InE 'INPUT_COMPAT_PATCH|compat_patch|auto-installer/patches' \
	auto-installer/runtime/kait2en-prepare
grep -Fq '"$transition_source" "$target_kernel" "$work/rpm"' \
	auto-installer/runtime/kait2en-prepare
# The transition modules only live in the initramfs, so they must be forced in.
grep -Fq 'dracut --force --force-drivers' \
	auto-installer/runtime/kait2en-prepare
! grep -Fq -- '--add-drivers' auto-installer/runtime/kait2en-prepare

# DKMS drops every kernel's build before rebuilding any of them, so a kernel
# without headers has to be refused before anything is removed.
grep -Fq 'require_kernel_headers' scripts/fedora/install-dkms-modules.sh
grep -Fq 'require_kernel_headers()' scripts/fedora/lib.sh
grep -Fq 't2-kernel-builder installation failed; continuing because it is optional' \
	scripts/fedora/install-apps.sh
grep -Fq 't2-kernel-builder bundle is incomplete' \
	apps/t2-kernel-builder/install.sh
# A wrong origin URL must be repaired; dying leaves the user with no way out.
grep -Fq 'repair_origin_url' auto-installer/runtime/kait2en-install
grep -Fq 'repair_origin_url' auto-installer/runtime/kait2en-prepare
! grep -Fq 'unexpected origin URL' \
	auto-installer/runtime/kait2en-install \
	auto-installer/runtime/kait2en-prepare
grep -Fq '["dkms", "autoinstall", "-k", release]' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
grep -Fq "DKMS_OVERRIDE.write_text('post_transaction=\"\"\\n'" \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
grep -Fq 'rebuilt initramfs is missing required module' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
for module in acpi_tad applesmc macsmc_core macsmc_acpi macsmc_hwmon \
		macsmc_light macsmc_accel leds_macsmc macsmc_chamshell rtc_macsmc \
		macsmc_power hid_apple hid_appletb_bl hid_appletb_kbd hid_magicmouse \
		appletbdrm apple_mfi_fastcharge apple_gmux t2bce_dma t2bce_core \
		t2bce_vhci t2hid; do
	grep -Fq "\"$module\"" apps/t2-kernel-builder/t2-kernel-builder-cleanup
done
grep -Fq 'installation rolled back' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
grep -Fq 'Cancelling installation and rolling back' \
	apps/t2-kernel-builder/t2-kernel-builder.py
grep -Fq 'Path(f"/boot/initramfs-{release}.img").is_file()' \
	apps/t2-kernel-builder/t2-kernel-builder.py
! grep -Fq '["dracut", "--force"' \
	apps/t2-kernel-builder/t2-kernel-builder-cleanup
for symbol in MFD_MACSMC_CORE MACSMC_ACPI SENSORS_MACSMC_HWMON \
		MACSMC_LIGHT MACSMC_ACCEL LEDS_MACSMC INPUT_MACSMC_CHAMSHELL \
		RTC_DRV_MACSMC MACSMC_POWER; do
	grep -Fq "$symbol" apps/t2-kernel-builder/engine/build.sh
done
grep -Fq 'required T2 kernel module was rejected by Kconfig' \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq -- '--enable IIO' apps/t2-kernel-builder/engine/build.sh
grep -Fq "grep -Eq '^\\+config[[:space:]]+MACSMC_ACPI" \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq 'if ((HAS_MACSMC_PATCHES)); then' \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq 'if ((HAS_AMD_DGPU)); then' \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq 'T2_REQUIRED_MODULES+=("${MACSMC_REQUIRED_MODULES[@]}")' \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq 'T2_REQUIRED_MODULES+=("${AMD_DGPU_REQUIRED_MODULES[@]}")' \
	apps/t2-kernel-builder/engine/build.sh
grep -Fq '/etc/modprobe.d/kait2en-silent-blacklist.conf' \
	scripts/fedora/install-kernel-args.sh
grep -Fq 'ln -sfn kait2en-multicall /usr/local/bin/edit-grub' scripts/fedora/install.sh
grep -Fq 'ln -sfn kait2en-multicall /usr/local/bin/update-grub' scripts/fedora/install.sh
grep -Fq 'exec sudo nano /etc/default/grub' scripts/fedora/kait2en-multicall
grep -Fq 'exec sudo grub2-mkconfig -o /boot/grub2/grub.cfg' \
	scripts/fedora/kait2en-multicall
grep -Fq "printf 'install %s /bin/true" scripts/fedora/install-kernel-args.sh
grep -Fq '"etc", "xdg", "autostart"' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
! grep -InE 'find_regular_user|home\.lstrip|os\.chown' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
grep -Fq 'KAIT2EN_AUTOSTART_FILE:-/etc/xdg/autostart/kait2en-install.desktop' \
	auto-installer/runtime/kait2en-prepare
! grep -InE '\$HOME/\.config/autostart' \
	auto-installer/runtime/kait2en-install
if grep -rInE --exclude-dir=target 'kait2en-first-boot|KAIT2EN_FIRST_BOOT' \
		auto-installer; then
	exit 1
fi
# The live Wi-Fi helpers must ride along in the input initramfs and must stay
# inside /run, which never reaches the installed system.
grep -Fq 'usr/lib/kait2en/kait2en-live-wifi' auto-installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-wifi.service' \
	auto-installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/install-wifi-firmware.sh' \
	auto-installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-diagnostics' \
	auto-installer/build-in-container.sh
grep -Fq 'runtime_units=/run/systemd/system' \
	auto-installer/initramfs/90-kait2en-updates.sh
grep -Fq 'ExecStart=/run/kait2en/kait2en-live-wifi' \
	auto-installer/runtime/kait2en-live-wifi.service
! grep -rInE --exclude-dir=target 'kait2en-live-wifi' \
	auto-installer/anaconda-addon

grep -Fq 'usr/lib/kait2en/kait2en-rescue' auto-installer/build-in-container.sh
grep -Fq 'kait2en-rescue' auto-installer/initramfs/90-kait2en-updates.sh
grep -Fq '/sysroot/usr/bin/kait2en-rescue' \
	auto-installer/initramfs/90-kait2en-updates.sh
grep -Fq 'apfs' auto-installer/runtime/kait2en-rescue
grep -Fq 'set-default-index' auto-installer/runtime/kait2en-rescue
! grep -Fq -- '--set-default ' auto-installer/runtime/kait2en-rescue
! grep -rInE --exclude-dir=target 'kait2en-rescue' \
	auto-installer/anaconda-addon

# Bluetooth firmware is loaded from disk by BCM4377 alone. Every entry point has
# to check for that PCI function, and the UART .hcd path must stay out of here.
grep -Fq '0x5fa0' auto-installer/runtime/install-bt-firmware.sh
grep -Fq '0x5fa0' auto-installer/runtime/kait2en-live-bluetooth
grep -Fq '0x5fa0' \
	auto-installer/anaconda-addon/com_kait2en_input/service/installation.py
grep -Fq 'BCM4377' scripts/macos/prepare-fedora-installer.sh
! grep -rInE --exclude-dir=target '\.hcd' auto-installer scripts/macos
grep -Fq 'usr/lib/kait2en/install-bt-firmware.sh' \
	auto-installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-bluetooth' \
	auto-installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-bluetooth.service' \
	auto-installer/build-in-container.sh
grep -Fq 'ExecStart=/run/kait2en/kait2en-live-bluetooth' \
	auto-installer/runtime/kait2en-live-bluetooth.service
! grep -rInE --exclude-dir=target 'kait2en-live-bluetooth' \
	auto-installer/anaconda-addon

grep -Fq 'Do not close this window!' auto-installer/runtime/kait2en-install
grep -Fq 'Ensure that you are connected to Wi-Fi before continuing.' \
	auto-installer/runtime/kait2en-install
grep -Fq 'Press any key to continue.' auto-installer/runtime/kait2en-install

# macOS Bash 3.2 treats an expanded empty array as unbound under `set -u`.
grep -Fq 'ORIGINAL_ARGC=$#' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'if ((ORIGINAL_ARGC == 0)); then' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'plist_value "$disk" WholeDisk' scripts/macos/prepare-fedora-installer.sh
! grep -Fq 'plist_value "$disk" Whole ' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'The ISO was verified OK.' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'Next steps:' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'Select the orange EFI Boot entry for this USB drive.' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'The KaiT2en installation will continue automatically in a terminal.' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'Good: Secure Boot has been disabled.' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'Set Secure Boot to No Security.' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'Allow booting from external or removable media.' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'reconnect the USB drive and retry with --reuse-media' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'source "$SCRIPT_DIR/download-fedora-iso.sh"' \
	scripts/macos/prepare-fedora-installer.sh
grep -Fq 'scripts/macos/download-fedora-iso.sh' \
	auto-installer/build-in-container.sh
grep -Fq 'FEDORA_METALINK=' auto-installer/targets/fedora-44.conf
grep -Fq 'FEDORA_ARCHIVE_BASEURL=' auto-installer/targets/fedora-44.conf
! grep -rIn 'FEDORA_BASEURL\|dl.fedoraproject.org/pub/fedora/linux/releases' \
	auto-installer/targets scripts/fedora auto-installer/build-in-container.sh
! grep -Fq 'Keep no second driver disk connected' scripts/macos/prepare-fedora-installer.sh
! grep -Fq 'before the intentional EFI customization' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'shasum -a 256 -c' auto-installer/macos-release-bootstrap.sh.in
grep -Fq 'KAIT2EN_TTY:-/dev/tty' auto-installer/macos-release-bootstrap.sh.in
[[ $(grep -Fc 'uses: actions/checkout@v5' .github/workflows/installer.yml) -eq 3 ]]
grep -Fq 'uses: actions/upload-artifact@v6' .github/workflows/installer.yml
grep -Fq 'uses: actions/download-artifact@v7' .github/workflows/installer.yml
! grep -InE 'uses: actions/(checkout|upload-artifact|download-artifact)@v4' \
	.github/workflows/installer.yml

patch_name=$(
	# shellcheck disable=SC1091
	source auto-installer/targets/fedora-44.conf
	printf '%s\n' "$INPUT_COMPAT_PATCH"
)
[[ -f "auto-installer/patches/$patch_name" ]]

git apply --unidiff-zero --check "auto-installer/patches/$patch_name"

bash scripts/tests/wifi-firmware.sh
bash scripts/tests/bt-firmware.sh
bash scripts/tests/live-wifi.sh
bash scripts/tests/live-bluetooth.sh
bash scripts/tests/rescue.sh
bash scripts/tests/prepare-install.sh
bash scripts/tests/install-launcher.sh
bash scripts/tests/release-bootstrap.sh
bash scripts/tests/terminal-launcher.sh
bash scripts/tests/iso-download.sh
bash scripts/tests/edition-catalog.sh
printf 'Installer static checks passed.\n'
