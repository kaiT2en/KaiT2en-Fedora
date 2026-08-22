#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command awk depmod dnf dracut find modinfo rm rpm sha256sum

usage() {
	printf 'Usage: %s [install|remove] [KERNEL_RELEASE] [--defer-initramfs]\n' "$0" >&2
	exit 2
}

ACTION=${1:-install}
shift $(( $# > 0 ? 1 : 0 ))
KVER=$(kernel_release)
KVER_SET=0
DEFER_INITRAMFS=0
for argument in "$@"; do
	case "$argument" in
		--defer-initramfs) DEFER_INITRAMFS=1 ;;
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
DRACUT_CONF="/etc/dracut.conf.d/90-kait2en-gpu-runtime-pm.conf"
BUILD_ID_FILE="$MODULE_DIR/.build-id"
PATCH_DIR="$REPO_ROOT/patches/runtime/gpu-runtime-pm"
PATCH_SERIES="$PATCH_DIR/series"
PATCH_FILES=()

is_supported_model() {
	local model

	[[ -r /sys/class/dmi/id/product_name ]] || return 1
	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBookPro15,1|MacBookPro15,3|MacBookPro16,1|MacBookPro16,4) return 0 ;;
		*) return 1 ;;
	esac
}

remove_modules() {
	if [[ -d "$MODULE_DIR" ]]; then
		info "removing GPU runtime PM modules for $KVER"
		rm -rf "$MODULE_DIR"
	fi
	rm -f "$MODPROBE_CONF"
	rm -f "$DRACUT_CONF"
	depmod -a "$KVER"
	if [[ -d "/usr/lib/modules/$KVER" ]]; then
		dracut --force "/boot/initramfs-$KVER.img" "$KVER"
	fi
}

apply_patch_if_needed() {
	local tree=$1 patch_file=$2

	if patch -d "$tree" -p1 --dry-run --batch --forward --fuzz=3 \
			--no-backup-if-mismatch \
			<"$patch_file" >/dev/null 2>&1; then
		info "applying ${patch_file#"$REPO_ROOT/"}"
		patch -d "$tree" -p1 --batch --forward --fuzz=3 \
			--no-backup-if-mismatch <"$patch_file"
	elif patch -d "$tree" -p1 --dry-run --batch --reverse --fuzz=3 \
			--no-backup-if-mismatch \
			<"$patch_file" >/dev/null 2>&1; then
		info "${patch_file#"$REPO_ROOT/"} is already present"
	else
		fail "patch does not apply to $KVER: ${patch_file#"$REPO_ROOT/"}"
	fi
}

load_patch_series() {
	local entry

	[[ -r "$PATCH_SERIES" ]] || fail "patch series is missing: ${PATCH_SERIES#"$REPO_ROOT/"}"
	while IFS= read -r entry || [[ -n "$entry" ]]; do
		[[ -n "$entry" && "$entry" != \#* ]] || continue
		[[ "$entry" != */* && "$entry" != .* ]] ||
			fail "invalid patch series entry: $entry"
		[[ -f "$PATCH_DIR/$entry" ]] ||
			fail "patch listed in series is missing: $entry"
		PATCH_FILES+=("$PATCH_DIR/$entry")
	done <"$PATCH_SERIES"
	((${#PATCH_FILES[@]} > 0)) || fail "patch series is empty"
}

case "$ACTION" in
	remove)
		require_command dracut rm
		remove_modules
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
build_id=$(
	sha256sum "$PATCH_SERIES" "${PATCH_FILES[@]}" |
		sha256sum | awk '{ print $1 }'
)
if [[ -f "$MODULE_DIR/amdgpu.ko.xz" &&
	-f "$MODULE_DIR/snd-hda-intel.ko.xz" &&
	-r "$BUILD_ID_FILE" &&
	$(<"$BUILD_ID_FILE") == "$build_id" ]]; then
	info "GPU runtime PM modules are current for $KVER"
	exit 0
fi

info "installing build dependencies for $KVER"
dnf install -y \
	"kernel-devel-$KVER" \
	cpio \
	curl \
	elfutils-libelf-devel \
	gcc \
	git-core \
	make \
	patch \
	tar \
	xz
require_command cpio curl git install make nproc patch rpm2cpio sed tar xz

[[ -d "/usr/src/kernels/$KVER" ]] || fail "kernel-devel is unavailable for $KVER"

if ! modinfo -k "$KVER" t2gmux >/dev/null 2>&1; then
	fail "t2gmux is not installed for $KVER"
fi

source_rpm=$(rpm -q --qf '%{SOURCERPM}\n' "kernel-core-$KVER") ||
	fail "kernel-core-$KVER is not installed"
source_name=${source_rpm%%-*}
source_version=$(rpm -q --qf '%{VERSION}\n' "kernel-core-$KVER")
source_release=$(rpm -q --qf '%{RELEASE}\n' "kernel-core-$KVER")
source_url="https://kojipkgs.fedoraproject.org/packages/$source_name/$source_version/$source_release/src/$source_rpm"

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT
source_dir="$workdir/sources"
download_dir="$workdir/download"
mkdir -p "$source_dir" "$download_dir" "$workdir/kernel"

info "downloading $source_rpm"
downloaded_srpm="$download_dir/$source_rpm"
curl --fail --location --output "$downloaded_srpm" "$source_url" ||
	fail "unable to download $source_url"

info "extracting Fedora kernel sources"
(
	cd "$source_dir"
	rpm2cpio "$downloaded_srpm" | cpio -idm --quiet
)

kernel_tarball=$(find "$source_dir" -maxdepth 1 -type f -name 'linux-*.tar.xz' -print -quit)
redhat_patch=$(find "$source_dir" -maxdepth 1 -type f -name 'patch-*-redhat.patch' -print -quit)
[[ -n "$kernel_tarball" ]] || fail "upstream kernel tarball is missing from $source_rpm"
[[ -n "$redhat_patch" ]] || fail "Fedora kernel patch is missing from $source_rpm"

info "preparing the Fedora kernel sources for $KVER"
tar -xf "$kernel_tarball" -C "$workdir/kernel"
amdgpu_source=$(find "$workdir/kernel" -type f \
	-path '*/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c' -print -quit)
[[ -n "$amdgpu_source" ]] || fail "kernel source tree was not found"
kernel_tree=${amdgpu_source%/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c}
git -C "$kernel_tree" apply "$redhat_patch" ||
	fail "Fedora kernel patch does not apply to its upstream source"

for patch_file in "${PATCH_FILES[@]}"; do
	apply_patch_if_needed "$kernel_tree" "$patch_file"
done

trace_header="$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu_trace.h"
grep -q '^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$' "$trace_header" ||
	fail "unexpected AMDGPU trace include path"
sed -i 's|^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$|#define TRACE_INCLUDE_PATH .|' \
	"$trace_header"

info "building AMDGPU for $KVER"
make -j "$(nproc)" -C "/usr/src/kernels/$KVER" \
	M="$kernel_tree/drivers/gpu/drm/amd/amdgpu" modules

info "building Intel HDA for $KVER"
make -j "$(nproc)" -C "/usr/src/kernels/$KVER" \
	M="$kernel_tree/sound/hda/controllers" modules

amdgpu_module="$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu.ko"
hda_module="$kernel_tree/sound/hda/controllers/snd-hda-intel.ko"
[[ -f "$amdgpu_module" && -f "$hda_module" ]] ||
	fail "one or more expected modules were not built"

staging="$workdir/modules"
install -Dpm 0644 "$amdgpu_module" "$staging/amdgpu.ko"
install -Dpm 0644 "$hda_module" "$staging/snd-hda-intel.ko"
xz --check=crc32 --lzma2=dict=1MiB -f \
	"$staging/amdgpu.ko" "$staging/snd-hda-intel.ko"

info "installing GPU runtime PM modules for $KVER"
install -d -m 0755 "$MODULE_DIR"
install -m 0644 "$staging/amdgpu.ko.xz" "$staging/snd-hda-intel.ko.xz" "$MODULE_DIR/"
cat >"$workdir/kait2en-gpu-runtime-pm.conf" <<'EOF'
# GMUX must provide the power callbacks before AMDGPU probes.
softdep amdgpu pre: t2gmux
EOF
install -Dpm 0644 "$workdir/kait2en-gpu-runtime-pm.conf" "$MODPROBE_CONF"
printf 'omit_drivers+=" snd_hda_intel "\n' >"$workdir/90-kait2en-gpu-runtime-pm.conf"
install -Dpm 0644 "$workdir/90-kait2en-gpu-runtime-pm.conf" "$DRACUT_CONF"
depmod -a "$KVER"
if ((DEFER_INITRAMFS == 0)); then
	dracut --force "/boot/initramfs-$KVER.img" "$KVER"
fi
printf '%s\n' "$build_id" >"$BUILD_ID_FILE"

info "GPU runtime PM modules installed for $KVER"
info "reboot into $KVER, then verify with: modinfo -n amdgpu"
