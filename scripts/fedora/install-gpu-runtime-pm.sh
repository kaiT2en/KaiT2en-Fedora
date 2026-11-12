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
MODULE_DIR="/usr/lib/modules/$KVER/updates/kait2en-gpu-runtime-pm"
MODPROBE_CONF="/usr/lib/modprobe.d/kait2en-gpu-runtime-pm.conf"
LEGACY_DRACUT_CONF="/etc/dracut.conf.d/90-kait2en-gpu-runtime-pm.conf"
HOOK_HOME="/usr/local/libexec/kait2en/gpu-runtime-pm"
KERNEL_INSTALL_HOOK="/etc/kernel/install.d/45-kait2en-gpu-runtime-pm.install"
if [[ -f "$SCRIPT_DIR/patches/series" ]]; then
	# The copy the kernel-install hook runs carries its own patches.
	PATCH_DIR="$SCRIPT_DIR/patches"
	FROM_REPO=0
else
	require_repo_root
	PATCH_DIR="$REPO_ROOT/patches/runtime/gpu-runtime-pm"
	FROM_REPO=1
fi
PATCH_SERIES="$PATCH_DIR/series"
PATCH_FILES=()

is_supported_model() {
	local model

	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBookPro15,1|MacBookPro16,1|MacBookPro16,4) return 0 ;;
		# No hybrid graphics, but their 5K panel needs the same patched AMDGPU.
		iMac20,1|iMac20,2|iMacPro1,1) return 0 ;;
		*) return 1 ;;
	esac
}

remove_modules() {
	if [[ -d "$MODULE_DIR" ]]; then
		info "removing GPU runtime PM modules for $KVER"
		rm -rf "$MODULE_DIR"
	fi
	rm -f "$MODPROBE_CONF"
	rm -f "$LEGACY_DRACUT_CONF"
	depmod -a "$KVER"
	if [[ -d "/usr/lib/modules/$KVER" ]]; then
		dracut --force "/boot/initramfs-$KVER.img" "$KVER"
	fi
}

remove_kernel_hook() {
	rm -f "$KERNEL_INSTALL_HOOK"
	rm -rf "$HOOK_HOME"
}

# Rebuild AMDGPU for every kernel dnf installs later: after 40-dkms has
# built t2gmux, before 50-depmod and 50-dracut pick the module up. The
# hook never fails the transaction and cannot call dnf from inside it.
install_kernel_hook() {
	local entry

	rm -rf "$HOOK_HOME"
	install -d -m 0755 "$HOOK_HOME/patches"
	install -m 0755 "$SCRIPT_DIR/install-gpu-runtime-pm.sh" "$HOOK_HOME/"
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
log=/var/log/kait2en-gpu-runtime-pm.log

[[ -n "\$kernelver" ]] || exit 0
case "\$command" in
	add)
		printf '[kait2en] building GPU runtime PM AMDGPU for %s\\n' "\$kernelver"
		if ! "$HOOK_HOME/install-gpu-runtime-pm.sh" install "\$kernelver" \\
				--defer-initramfs --no-deps >>"\$log" 2>&1; then
			printf '[kait2en] GPU runtime PM build for %s failed, see %s\\n' \\
				"\$kernelver" "\$log"
			printf '[kait2en] retry with: sudo %s install %s\\n' \\
				"$HOOK_HOME/install-gpu-runtime-pm.sh" "\$kernelver"
		fi
		;;
	remove)
		rm -rf "/usr/lib/modules/\$kernelver/updates/kait2en-gpu-runtime-pm"
		;;
esac
exit 0
HOOK
	chmod 0755 "$KERNEL_INSTALL_HOOK"
}

case "$ACTION" in
	remove)
		require_command dracut rm
		remove_modules
		remove_kernel_hook
		exit 0
		;;
	install) ;;
	*) usage ;;
esac

if ! is_supported_model; then
	info "GPU runtime PM is not supported on this model, skipping"
	exit 0
fi

load_patch_series

info "installing build dependencies for $KVER"
build_tree=$(readlink -f "/lib/modules/$KVER/build" || true)
if [[ -n "$build_tree" && -f "$build_tree/Makefile" &&
	-f "$build_tree/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c" ]]; then
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

# Inside kernel-install, 50-depmod has not indexed the DKMS modules yet.
if ! modinfo -k "$KVER" t2gmux >/dev/null 2>&1 &&
	[[ -z $(find "/usr/lib/modules/$KVER" -name 't2gmux.ko*' -print -quit) ]]; then
	fail "t2gmux is not installed for $KVER"
fi

info "using /var/tmp for kernel sources and module build"
workdir=$(mktemp -d /var/tmp/kait2en-gpu-runtime-pm.XXXXXXXXXX)
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
	[[ -f "$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c" ]] ||
		fail "kernel source tree was not found"
fi

for patch_file in "${PATCH_FILES[@]}"; do
	apply_patch_if_needed "$kernel_tree" "$patch_file"
done

trace_header="$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu_trace.h"
# A local kernel tree keeps the rewritten path from an earlier run.
grep -q -e '^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$' \
	-e '^#define TRACE_INCLUDE_PATH \.$' "$trace_header" ||
	fail "unexpected AMDGPU trace include path"
sed -i 's|^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$|#define TRACE_INCLUDE_PATH .|' \
	"$trace_header"

info "building AMDGPU for $KVER"
make -j "$(nproc)" -C "$build_tree" \
	M="$kernel_tree/drivers/gpu/drm/amd/amdgpu" modules

amdgpu_module="$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu.ko"
[[ -f "$amdgpu_module" ]] || fail "AMDGPU module was not built"

staging="$workdir/modules"
install -Dpm 0644 "$amdgpu_module" "$staging/amdgpu.ko"
strip --strip-debug "$staging/amdgpu.ko" ||
	warn "could not strip debug info; installing larger, unstripped modules"
xz --check=crc32 --lzma2=dict=1MiB -f "$staging/amdgpu.ko"

info "installing GPU runtime PM modules for $KVER"
install -d -m 0755 "$MODULE_DIR"
install -m 0644 "$staging/amdgpu.ko.xz" "$MODULE_DIR/"
rm -f "$MODULE_DIR/snd-hda-intel.ko.xz" "$MODULE_DIR/.build-id" "$LEGACY_DRACUT_CONF"
cat >"$workdir/kait2en-gpu-runtime-pm.conf" <<'EOF'
# GMUX must provide the power callbacks before AMDGPU probes.
softdep amdgpu pre: t2gmux
EOF
install -Dpm 0644 "$workdir/kait2en-gpu-runtime-pm.conf" "$MODPROBE_CONF"
depmod -a "$KVER"
if ((DEFER_INITRAMFS == 0)); then
	dracut --force "/boot/initramfs-$KVER.img" "$KVER"
fi

((FROM_REPO == 0)) || install_kernel_hook

info "GPU runtime PM modules installed for $KVER"
info "reboot into $KVER, then verify with: modinfo -n amdgpu"
