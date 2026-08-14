#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd "$repo_root"

shell_files=(
	packaging/installer/build-in-container.sh
	packaging/installer/build-input-kmod.sh
	packaging/installer/initramfs/20-kait2en-input.sh.in
	packaging/installer/initramfs/90-kait2en-updates.sh
	packaging/installer/macos-release-bootstrap.sh.in
	packaging/installer/runtime/install-bt-firmware.sh
	packaging/installer/runtime/install-wifi-firmware.sh
	packaging/installer/runtime/kait2en-install
	packaging/installer/runtime/kait2en-launch-terminal
	packaging/installer/runtime/kait2en-live-bluetooth
	packaging/installer/runtime/kait2en-live-diagnostics
	packaging/installer/runtime/kait2en-live-wifi
	packaging/installer/runtime/kait2en-prepare
	scripts/fedora/build-installer.sh
	scripts/fedora/install-dkms-modules.sh
	scripts/fedora/lib.sh
	scripts/fedora/rebuild-initramfs.sh
	scripts/macos/download-fedora-iso.sh
	scripts/macos/prepare-fedora-installer.sh
	scripts/tests/edition-catalog.sh
	scripts/tests/iso-download.sh
	scripts/tests/install-launcher.sh
	scripts/tests/static-check.sh
	scripts/tests/prepare-install.sh
	scripts/tests/release-bootstrap.sh
	scripts/tests/bt-firmware.sh
	scripts/tests/live-bluetooth.sh
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
done < <(git ls-files 'packaging/installer/anaconda-addon/*.py' \
	'packaging/installer/anaconda-addon/**/*.py')

! grep -rInE --exclude-dir=target 'OEMDRV|rhdd3|inst\.dd=|inst\.ks=|kait2en\.wifi_required' \
	packaging/installer/grub.cfg.in \
	packaging/installer/initramfs \
	packaging/installer/anaconda-addon \
	scripts/macos
! grep -rInE --exclude-dir=target 'brcmfmac(4364|4377).*alias|generic.*brcmfmac|brcmfmac[^ ]*-pcie\.txt' \
	packaging/installer
# A KaiT2en entry without the input initramfs loses the keyboard it rescues.
awk '
$1 == "linux" && index($0, "${kait2en_common}") { entries++ }
$1 == "initrd" && $2 == "${kait2en_initrd}" { overlays++ }
END { exit !(entries >= 5 && entries == overlays) }
' packaging/installer/grub.cfg.in
grep -Fq 'plymouth.enable=0' packaging/installer/grub.cfg.in
grep -Fq 'nomodeset' packaging/installer/grub.cfg.in
if grep -Eq '^set kait2en_blacklist=.*apple_gmux' packaging/installer/grub.cfg.in; then
	exit 1
fi
! grep -InE 'INPUT_COMPAT_PATCH|compat_patch|packaging/installer/patches' \
	packaging/installer/runtime/kait2en-prepare
grep -Fq '"$transition_source" "$target_kernel" "$work/rpm"' \
	packaging/installer/runtime/kait2en-prepare
# The transition modules only live in the initramfs, so they must be forced in.
grep -Fq 'dracut --force --force-drivers' \
	packaging/installer/runtime/kait2en-prepare
! grep -Fq -- '--add-drivers' packaging/installer/runtime/kait2en-prepare

grep -Fq 'force_drivers' scripts/fedora/rebuild-initramfs.sh

# DKMS drops every kernel's build before rebuilding any of them, so a kernel
# without headers has to be refused before anything is removed.
grep -Fq 'require_kernel_headers' scripts/fedora/install-dkms-modules.sh
grep -Fq 'require_kernel_headers()' scripts/fedora/lib.sh
grep -Fq '"etc", "xdg", "autostart"' \
	packaging/installer/anaconda-addon/com_kait2en_input/service/installation.py
! grep -InE 'find_regular_user|home\.lstrip|os\.chown' \
	packaging/installer/anaconda-addon/com_kait2en_input/service/installation.py
grep -Fq 'KAIT2EN_AUTOSTART_FILE:-/etc/xdg/autostart/kait2en-install.desktop' \
	packaging/installer/runtime/kait2en-prepare
! grep -InE '\$HOME/\.config/autostart' \
	packaging/installer/runtime/kait2en-install
if grep -rInE --exclude-dir=target 'kait2en-first-boot|KAIT2EN_FIRST_BOOT' \
		packaging/installer; then
	exit 1
fi
# The live Wi-Fi helpers must ride along in the input initramfs and must stay
# inside /run, which never reaches the installed system.
grep -Fq 'usr/lib/kait2en/kait2en-live-wifi' packaging/installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-wifi.service' \
	packaging/installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/install-wifi-firmware.sh' \
	packaging/installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-diagnostics' \
	packaging/installer/build-in-container.sh
grep -Fq 'runtime_units=/run/systemd/system' \
	packaging/installer/initramfs/90-kait2en-updates.sh
grep -Fq 'ExecStart=/run/kait2en/kait2en-live-wifi' \
	packaging/installer/runtime/kait2en-live-wifi.service
! grep -rInE --exclude-dir=target 'kait2en-live-wifi' \
	packaging/installer/anaconda-addon

# Bluetooth firmware is loaded from disk by BCM4377 alone. Every entry point has
# to check for that PCI function, and the UART .hcd path must stay out of here.
grep -Fq '0x5fa0' packaging/installer/runtime/install-bt-firmware.sh
grep -Fq '0x5fa0' packaging/installer/runtime/kait2en-live-bluetooth
grep -Fq '0x5fa0' \
	packaging/installer/anaconda-addon/com_kait2en_input/service/installation.py
grep -Fq 'BCM4377' scripts/macos/prepare-fedora-installer.sh
! grep -rInE --exclude-dir=target '\.hcd' packaging/installer scripts/macos
grep -Fq 'usr/lib/kait2en/install-bt-firmware.sh' \
	packaging/installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-bluetooth' \
	packaging/installer/build-in-container.sh
grep -Fq 'usr/lib/kait2en/kait2en-live-bluetooth.service' \
	packaging/installer/build-in-container.sh
grep -Fq 'ExecStart=/run/kait2en/kait2en-live-bluetooth' \
	packaging/installer/runtime/kait2en-live-bluetooth.service
! grep -rInE --exclude-dir=target 'kait2en-live-bluetooth' \
	packaging/installer/anaconda-addon

grep -Fq 'Do not close this window!' packaging/installer/runtime/kait2en-install
grep -Fq 'Ensure that you are connected to Wi-Fi before continuing.' \
	packaging/installer/runtime/kait2en-install
grep -Fq 'Press any key to continue.' packaging/installer/runtime/kait2en-install

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
	packaging/installer/build-in-container.sh
grep -Fq 'FEDORA_METALINK=' packaging/installer/targets/fedora-44.conf
grep -Fq 'FEDORA_ARCHIVE_BASEURL=' packaging/installer/targets/fedora-44.conf
! grep -rIn 'FEDORA_BASEURL\|dl.fedoraproject.org/pub/fedora/linux/releases' \
	packaging/installer/targets scripts/fedora packaging/installer/build-in-container.sh
! grep -Fq 'Keep no second driver disk connected' scripts/macos/prepare-fedora-installer.sh
! grep -Fq 'before the intentional EFI customization' scripts/macos/prepare-fedora-installer.sh
grep -Fq 'shasum -a 256 -c' packaging/installer/macos-release-bootstrap.sh.in
grep -Fq 'KAIT2EN_TTY:-/dev/tty' packaging/installer/macos-release-bootstrap.sh.in
[[ $(grep -Fc 'uses: actions/checkout@v5' .github/workflows/installer.yml) -eq 3 ]]
grep -Fq 'uses: actions/upload-artifact@v6' .github/workflows/installer.yml
grep -Fq 'uses: actions/download-artifact@v7' .github/workflows/installer.yml
! grep -InE 'uses: actions/(checkout|upload-artifact|download-artifact)@v4' \
	.github/workflows/installer.yml

patch_name=$(
	# shellcheck disable=SC1091
	source packaging/installer/targets/fedora-44.conf
	printf '%s\n' "$INPUT_COMPAT_PATCH"
)
[[ -f "packaging/installer/patches/$patch_name" ]]

git apply --unidiff-zero --check "packaging/installer/patches/$patch_name"

bash scripts/tests/wifi-firmware.sh
bash scripts/tests/bt-firmware.sh
bash scripts/tests/live-wifi.sh
bash scripts/tests/live-bluetooth.sh
bash scripts/tests/prepare-install.sh
bash scripts/tests/install-launcher.sh
bash scripts/tests/release-bootstrap.sh
bash scripts/tests/terminal-launcher.sh
bash scripts/tests/iso-download.sh
bash scripts/tests/edition-catalog.sh
printf 'Installer static checks passed.\n'
