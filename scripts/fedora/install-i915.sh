#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_fedora
require_command depmod dnf dracut find modinfo rm rpm

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
MODULE_DIR="/usr/lib/modules/$KVER/updates/kait2en-i915"
HOOK_HOME="/usr/local/libexec/kait2en/i915"
KERNEL_INSTALL_HOOK="/etc/kernel/install.d/47-kait2en-i915.install"
if [[ -f "$SCRIPT_DIR/patches/series" ]]; then
	# The copy the kernel-install hook runs carries its own patches.
	PATCH_DIR="$SCRIPT_DIR/patches"
	FROM_REPO=0
else
	require_repo_root
	PATCH_DIR="$REPO_ROOT/patches/runtime/i915"
	FROM_REPO=1
fi
PATCH_SERIES="$PATCH_DIR/series"
PATCH_FILES=()

# MacBooks and iMacs with an Intel GPU. Mac minis and Mac Pros have no
# internal panel for the PSR and backlight patches.
is_supported_model() {
	local model vendor class

	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBook*|iMac*) ;;
		*) return 1 ;;
	esac
	[[ -r /sys/bus/pci/devices/0000:00:02.0/vendor ]] || return 1
	read -r vendor </sys/bus/pci/devices/0000:00:02.0/vendor
	read -r class </sys/bus/pci/devices/0000:00:02.0/class
	[[ $vendor == 0x8086 && $class == 0x03* ]]
}

remove_modules() {
	if [[ -d "$MODULE_DIR" ]]; then
		info "removing patched i915 modules for $KVER"
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

# Rebuild i915 for every kernel dnf installs later, before 50-depmod and
# 50-dracut pick the modules up. The hook never fails the transaction and
# cannot call dnf from inside it.
install_kernel_hook() {
	local entry

	rm -rf "$HOOK_HOME"
	install -d -m 0755 "$HOOK_HOME/patches"
	install -m 0755 "$SCRIPT_DIR/install-i915.sh" "$HOOK_HOME/"
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
log=/var/log/kait2en-i915.log

[[ -n "\$kernelver" ]] || exit 0
case "\$command" in
	add)
		printf '[kait2en] building patched i915 for %s\\n' "\$kernelver"
		if ! "$HOOK_HOME/install-i915.sh" install "\$kernelver" \\
				--defer-initramfs --no-deps >>"\$log" 2>&1; then
			printf '[kait2en] i915 build for %s failed, see %s\\n' \\
				"\$kernelver" "\$log"
			printf '[kait2en] retry with: sudo %s install %s\\n' \\
				"$HOOK_HOME/install-i915.sh" "\$kernelver"
		fi
		;;
	remove)
		rm -rf "/usr/lib/modules/\$kernelver/updates/kait2en-i915"
		;;
esac
exit 0
HOOK
	chmod 0755 "$KERNEL_INSTALL_HOOK"
}

# Point a trace header at its own directory, which -I\$(src) resolves in an
# external module build. A local kernel tree keeps the rewrite of an earlier run.
rewrite_trace_path() {
	local header=$1 upstream=$2 local_path=$3

	grep -q -e "^#define TRACE_INCLUDE_PATH $upstream\$" \
		-e "^#define TRACE_INCLUDE_PATH $local_path\$" "$header" ||
		fail "unexpected trace include path in ${header##*/}"
	sed -i "s|^#define TRACE_INCLUDE_PATH $upstream\$|#define TRACE_INCLUDE_PATH $local_path|" \
		"$header"
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

if ! is_supported_model; then
	info "the patched i915 is not needed on this model, skipping"
	exit 0
fi

load_patch_series

info "installing build dependencies for $KVER"
build_tree=$(readlink -f "/lib/modules/$KVER/build" || true)
if [[ -n "$build_tree" && -f "$build_tree/Makefile" &&
	-f "$build_tree/drivers/gpu/drm/i915/i915_driver.c" ]]; then
	local_kernel_tree=1
	info "using local kernel build tree $build_tree"
	development_package=()
else
	local_kernel_tree=0
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
require_command cpio curl git install make nproc patch rpm2cpio sed strip tar xz

if ((local_kernel_tree == 0)); then
	build_tree="/usr/src/kernels/$KVER"
	[[ -d "$build_tree" ]] || fail "kernel-devel is unavailable for $KVER"
fi

info "using /var/tmp for kernel sources and module build"
workdir=$(mktemp -d /var/tmp/kait2en-i915.XXXXXXXXXX)
trap 'rm -rf "$workdir"' EXIT
if ((local_kernel_tree)); then
	kernel_tree=$build_tree
else
	pool_tree=$(kernel_source_tree "$KVER") ||
		fail "kernel source for $KVER is unavailable"
	# Reflinks on btrfs make the copy of the shared pool nearly free.
	cp -a --reflink=auto "$pool_tree" "$workdir/kernel" ||
		fail "unable to copy the kernel source for $KVER"
	kernel_tree="$workdir/kernel"
	[[ -f "$kernel_tree/drivers/gpu/drm/i915/i915_driver.c" ]] ||
		fail "kernel source tree was not found"
fi

for patch_file in "${PATCH_FILES[@]}"; do
	apply_patch_if_needed "$kernel_tree" "$patch_file"
done

i915_dir="$kernel_tree/drivers/gpu/drm/i915"
display_helper_dir="$kernel_tree/drivers/gpu/drm/display"
rewrite_trace_path "$i915_dir/i915_trace.h" ../../drivers/gpu/drm/i915 .
rewrite_trace_path "$i915_dir/intel_uncore_trace.h" ../../drivers/gpu/drm/i915 .
rewrite_trace_path "$i915_dir/display/intel_display_trace.h" \
	../../drivers/gpu/drm/i915/display display
rewrite_trace_path "$i915_dir/gvt/trace.h" ../../drivers/gpu/drm/i915/gvt gvt

# The Apple panel quirk lives in drm_display_helper, the PSR handshake and
# the gmux brightness restore in i915.
info "building drm_display_helper for $KVER"
make -j "$(nproc)" -C "$build_tree" M="$display_helper_dir" modules
info "building i915 for $KVER"
make -j "$(nproc)" -C "$build_tree" M="$i915_dir" modules

staging="$workdir/modules"
for module in "$display_helper_dir/drm_display_helper.ko" "$i915_dir/i915.ko"; do
	[[ -f "$module" ]] || fail "${module##*/} was not built"
	install -Dpm 0644 "$module" "$staging/${module##*/}"
	strip --strip-debug "$staging/${module##*/}" ||
		warn "could not strip debug info, installing a larger, unstripped ${module##*/}"
	xz --check=crc32 --lzma2=dict=1MiB -f "$staging/${module##*/}"
done

info "installing patched i915 modules for $KVER"
install -d -m 0755 "$MODULE_DIR"
install -m 0644 "$staging/drm_display_helper.ko.xz" "$staging/i915.ko.xz" "$MODULE_DIR/"
depmod -a "$KVER"
if ((DEFER_INITRAMFS == 0)); then
	dracut --force "/boot/initramfs-$KVER.img" "$KVER"
fi

((FROM_REPO == 0)) || install_kernel_hook

info "patched i915 modules installed for $KVER"
info "reboot into $KVER, then verify with: modinfo -n i915"
