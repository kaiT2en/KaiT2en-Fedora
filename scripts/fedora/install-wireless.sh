#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_fedora
require_command depmod dnf dracut find rm rpm

usage() {
	printf 'Usage: %s [install|remove] [KERNEL_RELEASE] [--defer-initramfs] [--no-deps]\n' "$0" >&2
	exit 2
}

ACTION=${1:-install}
shift $(( $# > 0 ? 1 : 0 ))
KVER=$(kernel_release)
KVER_SET=0
DEFER_INITRAMFS=0
SKIP_DEPS=0
for argument in "$@"; do
	case "$argument" in
		--defer-initramfs) DEFER_INITRAMFS=1 ;;
		--no-deps) SKIP_DEPS=1 ;;
		--*) usage ;;
		*)
			((KVER_SET == 0)) || usage
			KVER=$argument
			KVER_SET=1
			;;
	esac
done
MODULE_DIR="/usr/lib/modules/$KVER/updates/kait2en-wireless"
HOOK_HOME="/usr/local/libexec/kait2en/wireless"
KERNEL_INSTALL_HOOK="/etc/kernel/install.d/46-kait2en-wireless.install"
if [[ -f "$SCRIPT_DIR/patches/series" ]]; then
	# The copy the kernel-install hook runs carries its own patches.
	PATCH_DIR="$SCRIPT_DIR/patches"
	FROM_REPO=0
else
	require_repo_root
	PATCH_DIR="$REPO_ROOT/patches/runtime/wireless"
	FROM_REPO=1
fi
PATCH_SERIES="$PATCH_DIR/series"
PATCH_FILES=()

# brcmfmac covers the WLAN of every T2 Mac, hci_uart the UART Bluetooth of the
# BCM4355/BCM4364 Macs, hci_bcm4377 the PCIe Bluetooth. Both Bluetooth drivers
# link to btbcm. Only drivers the target kernel builds as modules are replaced.
BRCMFMAC_DIR=drivers/net/wireless/broadcom/brcm80211/brcmfmac
BRCMUTIL_DIR=drivers/net/wireless/broadcom/brcm80211/brcmutil
BLUETOOTH_DIR=drivers/bluetooth
MODULES=()
BLUETOOTH_MAKE_ARGS=()
BRCMFMAC_MAKE_ARGS=(CONFIG_BRCMDBG=y "KCPPFLAGS=-DDEBUG -DCONFIG_BRCMDBG=1")

kernel_config() {
	sed -n "s/^CONFIG_$1=//p" "$build_tree/.config"
}

select_modules() {
	if [[ $(kernel_config BRCMFMAC) == m ]]; then
		MODULES+=(
			"$BRCMUTIL_DIR/brcmutil.ko"
			"$BRCMFMAC_DIR/brcmfmac.ko"
			"$BRCMFMAC_DIR/wcc/brcmfmac-wcc.ko"
			"$BRCMFMAC_DIR/cyw/brcmfmac-cyw.ko"
			"$BRCMFMAC_DIR/bca/brcmfmac-bca.ko"
		)
	fi
	[[ $(kernel_config BT_HCIUART) == m ]] && MODULES+=("$BLUETOOTH_DIR/hci_uart.ko")
	[[ $(kernel_config BT_HCIBCM4377) == m ]] && MODULES+=("$BLUETOOTH_DIR/hci_bcm4377.ko")
	[[ " ${MODULES[*]} " == *" $BLUETOOTH_DIR/"* ]] || return 0

	case $(kernel_config BT_BCM) in
		m) ;;
		y) fail "btbcm is built into $KVER, the patched Bluetooth drivers cannot link to it" ;;
		*)
			# The patch makes BT_HCIBCM4377 select BT_BCM, a kernel configured
			# before it lacks btbcm, so it is built here as well.
			BLUETOOTH_MAKE_ARGS=(CONFIG_BT_BCM=m KCPPFLAGS=-DCONFIG_BT_BCM_MODULE=1)
			;;
	esac
	MODULES+=("$BLUETOOTH_DIR/btbcm.ko")
}

has_broadcom_wireless() {
	local device vendor class

	for device in /sys/bus/pci/devices/*; do
		[[ -r "$device/vendor" && -r "$device/class" ]] || continue
		read -r vendor <"$device/vendor"
		read -r class <"$device/class"
		[[ "$vendor" == 0x14e4 && "$class" == 0x0280* ]] && return 0
	done
	# UART Bluetooth of the BCM4355/BCM4364 Macs
	compgen -G '/sys/bus/acpi/devices/BCM2E7C*' >/dev/null
}

remove_modules() {
	if [[ -d "$MODULE_DIR" ]]; then
		info "removing wireless modules for $KVER"
		rm -rf "$MODULE_DIR"
	fi
	depmod -a "$KVER"
	if [[ -d "/usr/lib/modules/$KVER" ]]; then
		dracut --force "/boot/initramfs-$KVER.img" "$KVER"
	fi
}

remove_kernel_hook() {
	rm -f "$KERNEL_INSTALL_HOOK"
	rm -rf "$HOOK_HOME"
}

# Rebuild the wireless modules for every kernel dnf installs later: after
# 44-kait2en-kernel-source has prepared the source, before 50-depmod and
# 50-dracut. The hook never fails the transaction and cannot call dnf.
install_kernel_hook() {
	local entry

	rm -rf "$HOOK_HOME"
	install -d -m 0755 "$HOOK_HOME/patches"
	install -m 0755 "$SCRIPT_DIR/install-wireless.sh" "$HOOK_HOME/"
	install -m 0644 "$SCRIPT_DIR/lib.sh" "$HOOK_HOME/"
	install -m 0644 "$PATCH_SERIES" "$HOOK_HOME/patches/series"
	for entry in "${PATCH_FILES[@]}"; do
		install -m 0644 "$entry" "$HOOK_HOME/patches/"
	done

	install -d -m 0755 "${KERNEL_INSTALL_HOOK%/*}"
	cat >"$KERNEL_INSTALL_HOOK" <<HOOK
#!/usr/bin/bash

command=\${1:-}
kernelver=\${2:-}
log=/var/log/kait2en-wireless.log

[[ -n "\$kernelver" ]] || exit 0
case "\$command" in
	add)
		printf '[kait2en] building wireless modules for %s\\n' "\$kernelver"
		if ! "$HOOK_HOME/install-wireless.sh" install "\$kernelver" \\
				--defer-initramfs --no-deps >>"\$log" 2>&1; then
			printf '[kait2en] wireless build for %s failed, see %s\\n' \\
				"\$kernelver" "\$log"
			printf '[kait2en] retry with: sudo %s install %s\\n' \\
				"$HOOK_HOME/install-wireless.sh" "\$kernelver"
		fi
		;;
	remove)
		rm -rf "/usr/lib/modules/\$kernelver/updates/kait2en-wireless"
		;;
esac
exit 0
HOOK
	chmod 0755 "$KERNEL_INSTALL_HOOK"
}

case "$ACTION" in
	remove)
		remove_modules
		remove_kernel_hook
		exit 0
		;;
	install) ;;
	*) usage ;;
esac

if ! has_broadcom_wireless; then
	info "no Broadcom WLAN or Bluetooth found, skipping"
	exit 0
fi

load_patch_series

info "installing build dependencies for $KVER"
build_tree=$(readlink -f "/lib/modules/$KVER/build" || true)
if [[ -n "$build_tree" && -f "$build_tree/Makefile" && -f "$build_tree/init/main.c" ]]; then
	# A self-built kernel brings its full tree, there is no kernel-devel for it.
	info "using local kernel build tree $build_tree"
	development_package=()
else
	development_package=("kernel-devel-$KVER")
fi

((SKIP_DEPS)) || dnf install -y \
	"${development_package[@]}" \
	cpio \
	curl \
	elfutils-libelf-devel \
	gcc \
	git-core \
	make \
	patch \
	tar \
	xz
require_command cpio curl git install make nproc patch rpm2cpio strip tar xz

[[ -n "$build_tree" && -f "$build_tree/Makefile" ]] || build_tree="/usr/src/kernels/$KVER"
[[ -f "$build_tree/Makefile" ]] || fail "kernel-devel is unavailable for $KVER"
[[ -r "$build_tree/.config" ]] || fail "kernel configuration is missing in $build_tree"

select_modules
if ((${#MODULES[@]} == 0)); then
	info "$KVER builds none of the Broadcom wireless drivers as modules, skipping"
	exit 0
fi

source_tree=$(kernel_source_tree "$KVER") ||
	fail "kernel source for $KVER is unavailable"

info "using /var/tmp for the wireless module build"
workdir=$(mktemp -d /var/tmp/kait2en-wireless.XXXXXXXXXX)
trap 'rm -rf "$workdir"' EXIT
tree="$workdir/linux"
install -d "$tree/${BRCMFMAC_DIR%/*}" "$tree/${BLUETOOTH_DIR%/*}"
cp -a --reflink=auto "$source_tree/$BRCMFMAC_DIR" "$tree/${BRCMFMAC_DIR%/*}/" ||
	fail "brcmfmac source is missing in $source_tree"
cp -a --reflink=auto "$source_tree/$BRCMUTIL_DIR" "$tree/${BRCMUTIL_DIR%/*}/" ||
	fail "brcmutil source is missing in $source_tree"
cp -a --reflink=auto "$source_tree/${BRCMFMAC_DIR%/*}/include" "$tree/${BRCMFMAC_DIR%/*}/" ||
	fail "brcm80211 headers are missing in $source_tree"
cp -a --reflink=auto "$source_tree/$BLUETOOTH_DIR" "$tree/${BLUETOOTH_DIR%/*}/" ||
	fail "Bluetooth driver source is missing in $source_tree"

for patch_file in "${PATCH_FILES[@]}"; do
	apply_patch_if_needed "$tree" "$patch_file"
done

if [[ " ${MODULES[*]} " == *" $BRCMFMAC_DIR/"* ]]; then
	info "building brcmutil for $KVER"
	make -j "$(nproc)" -C "$build_tree" M="$tree/$BRCMUTIL_DIR" \
		"${BRCMFMAC_MAKE_ARGS[@]}" modules

	info "building brcmfmac for $KVER"
	make -j "$(nproc)" -C "$build_tree" M="$tree/$BRCMFMAC_DIR" \
		KBUILD_EXTRA_SYMBOLS="$tree/$BRCMUTIL_DIR/Module.symvers" \
		"${BRCMFMAC_MAKE_ARGS[@]}" modules
fi
if [[ " ${MODULES[*]} " == *" $BLUETOOTH_DIR/"* ]]; then
	info "building the Bluetooth drivers for $KVER"
	make -j "$(nproc)" -C "$build_tree" M="$tree/$BLUETOOTH_DIR" \
		"${BLUETOOTH_MAKE_ARGS[@]}" modules
fi

staging="$workdir/modules"
install -d "$staging"
for module in "${MODULES[@]}"; do
	[[ -f "$tree/$module" ]] || fail "${module##*/} was not built"
	install -pm 0644 "$tree/$module" "$staging/"
	strip --strip-debug "$staging/${module##*/}" ||
		warn "could not strip debug info from ${module##*/}; installing it unstripped"
	xz --check=crc32 --lzma2=dict=1MiB -f "$staging/${module##*/}"
done

info "installing wireless modules for $KVER"
rm -rf "$MODULE_DIR"
install -d -m 0755 "$MODULE_DIR"
install -m 0644 "$staging"/*.ko.xz "$MODULE_DIR/"
depmod -a "$KVER"
if ((DEFER_INITRAMFS == 0)); then
	dracut --force "/boot/initramfs-$KVER.img" "$KVER"
fi

((FROM_REPO == 0)) || install_kernel_hook

info "wireless modules installed for $KVER"
info "reboot into $KVER, then verify with: modinfo -n brcmfmac btbcm"
