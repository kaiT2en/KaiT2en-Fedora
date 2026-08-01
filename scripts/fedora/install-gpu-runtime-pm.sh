#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command depmod dnf dracut find modinfo rm rpm

ACTION=${1:-install}
KVER=${2:-$(kernel_release)}
MODULE_DIR="/usr/lib/modules/$KVER/updates/kait2en-gpu-runtime-pm"
MODPROBE_CONF="/usr/lib/modprobe.d/kait2en-gpu-runtime-pm.conf"
DRACUT_CONF="/etc/dracut.conf.d/90-kait2en-gpu-runtime-pm.conf"

usage() {
	printf 'Usage: %s [install|remove] [KERNEL_RELEASE]\n' "$0" >&2
	exit 2
}

require_supported_model() {
	local model

	[[ -r /sys/class/dmi/id/product_name ]] || fail "DMI product name is unavailable"
	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBookPro15,1|MacBookPro15,3|MacBookPro16,1|MacBookPro16,4) ;;
		*) fail "GPU runtime PM is not supported on $model" ;;
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
			<"$patch_file" >/dev/null 2>&1; then
		info "applying ${patch_file#"$REPO_ROOT/"}"
		patch -d "$tree" -p1 --batch --forward --fuzz=3 <"$patch_file"
	elif patch -d "$tree" -p1 --dry-run --batch --reverse --fuzz=3 \
			<"$patch_file" >/dev/null 2>&1; then
		info "${patch_file#"$REPO_ROOT/"} is already present"
	else
		fail "patch does not apply to $KVER: ${patch_file#"$REPO_ROOT/"}"
	fi
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

require_supported_model

info "installing build dependencies for $KVER"
dnf install -y \
	"kernel-devel-$KVER" \
	cpio \
	curl \
	elfutils-libelf-devel \
	gcc \
	make \
	patch \
	rpm-build \
	xz
require_command cpio curl install make patch rpm2cpio rpmbuild sed xz

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
topdir="$workdir/rpmbuild"
download_dir="$workdir/download"
mkdir -p "$topdir"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS} "$download_dir"

info "downloading $source_rpm"
downloaded_srpm="$download_dir/$source_rpm"
curl --fail --location --output "$downloaded_srpm" "$source_url" ||
	fail "unable to download $source_url"

info "extracting Fedora kernel sources"
(
	cd "$topdir"
	rpm2cpio "$downloaded_srpm" | cpio -idm --quiet
)

spec=$(find "$topdir" -maxdepth 1 -type f -name 'kernel*.spec' -print -quit)
[[ -n "$spec" ]] || fail "kernel spec file is missing from $source_rpm"
mv "$topdir"/*.spec "$topdir/SPECS/"
find "$topdir" -maxdepth 1 -type f -exec mv -t "$topdir/SOURCES" -- {} +
spec="$topdir/SPECS/$(basename "$spec")"

info "preparing the Fedora kernel tree for $KVER"
rpmbuild -bp --nodeps --define "_topdir $topdir" --target "$(uname -m)" "$spec"

amdgpu_source=$(find "$topdir/BUILD" -type f \
	-path '*/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c' -print -quit)
[[ -n "$amdgpu_source" ]] || fail "prepared Fedora kernel tree was not found"
kernel_tree=${amdgpu_source%/drivers/gpu/drm/amd/amdgpu/amdgpu_drv.c}

apply_patch_if_needed "$kernel_tree" \
	"$REPO_ROOT/patches/amdgpu/0001-drm-amdgpu-reset-VI-ASIC-on-MacBookPro15-1.patch"
apply_patch_if_needed "$kernel_tree" \
	"$REPO_ROOT/patches/amdgpu/0002-drm-amdgpu-Add-Apple-GMUX-runtime-PM-support.patch"
apply_patch_if_needed "$kernel_tree" \
	"$REPO_ROOT/patches/hda/0001-ALSA-hda-Allow-direct-complete-with-a-powered-off-GPU.patch"

trace_header="$kernel_tree/drivers/gpu/drm/amd/amdgpu/amdgpu_trace.h"
grep -q '^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$' "$trace_header" ||
	fail "unexpected AMDGPU trace include path"
sed -i 's|^#define TRACE_INCLUDE_PATH ../../drivers/gpu/drm/amd/amdgpu$|#define TRACE_INCLUDE_PATH .|' \
	"$trace_header"

info "building AMDGPU for $KVER"
make -C "/usr/src/kernels/$KVER" \
	M="$kernel_tree/drivers/gpu/drm/amd/amdgpu" modules

info "building Intel HDA for $KVER"
make -C "/usr/src/kernels/$KVER" \
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
dracut --force "/boot/initramfs-$KVER.img" "$KVER"

info "GPU runtime PM modules installed for $KVER"
info "reboot into $KVER, then verify with: modinfo -n amdgpu"
