#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
PATCH_DIR=$ROOT/patches
BUILD_ROOT=${KERNEL_BUILD_ROOT:-$ROOT/build}
LOCALVERSION=-patched
CONFIG_FILE=
KERNEL_RELEASE=
JOBS=$(nproc)
CLEAN=0
LOCAL_INSTALL=0
DEFER_INSTALL=0
T2_CONFIG=0
PREPARE_ONLY=0
LOCALMODCONFIG=0
ALLOW_NO_PATCHES=0
BUILD_CONFIG_SCHEMA=10
ENABLE_CONFIGS=()
HAS_AMD_DGPU=0
HAS_MACSMC_PATCHES=0
T2_REQUIRED_MODULES=(
	ACPI_TAD
	SENSORS_APPLESMC
	HID_APPLE
	HID_APPLETB_BL
	HID_APPLETB_KBD
	HID_MAGICMOUSE
	DRM_APPLETBDRM
	APPLE_MFI_FASTCHARGE
	DRM_I915
	USB4
	MEDIA_SUPPORT
	VIDEO_DEV
	VIDEO_VIM2M
)
AMD_DGPU_REQUIRED_MODULES=(
	APPLE_GMUX
	DRM_AMDGPU
	SND_HDA_INTEL
	SND_HDA_CODEC_HDMI
)
MACSMC_REQUIRED_MODULES=(
	MFD_MACSMC_CORE
	MACSMC_ACPI
	SENSORS_MACSMC_HWMON
	MACSMC_LIGHT
	MACSMC_ACCEL
	LEDS_MACSMC
	MACSMC_WDT
	RTC_DRV_MACSMC
	MACSMC_POWER
)

fail() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

usage() {
	cat <<EOF
Usage: $0 [KERNEL_RELEASE] [options]

Without KERNEL_RELEASE, an interactive list is loaded from Fedora Koji.

Options:
  --config FILE          Use this kernel configuration
  --patch-dir DIRECTORY  Apply patches from this directory (default: patches/)
  --localversion SUFFIX  Set a unique suffix (default: -patched)
  --jobs NUMBER          Parallel build jobs (default: $JOBS)
  --local-install        Build and install directly instead of creating RPMs
  --defer-install        Build local targets without installing them
  --t2-config            Disable drivers not used by Apple T2 Macs
  --localmodconfig       Reduce the configuration to currently loaded modules
  --allow-no-patches     Permit an empty patch folder (configuration preview)
  --enable-config NAME   Enable an additional Kconfig symbol (repeatable)
  --prepare-only         Download, patch and configure without building
  --clean                Remove this version's existing build first
EOF
	exit 2
}

if [[ $# -gt 0 && $1 != --* ]]; then
	KERNEL_RELEASE=$1
	shift
fi

while [[ $# -gt 0 ]]; do
	case $1 in
	--config)
		[[ $# -ge 2 ]] || usage
		CONFIG_FILE=$2
		shift 2
		;;
	--patch-dir)
		[[ $# -ge 2 ]] || usage
		PATCH_DIR=$2
		shift 2
		;;
	--localversion)
		[[ $# -ge 2 ]] || usage
		LOCALVERSION=$2
		shift 2
		;;
	--jobs)
		[[ $# -ge 2 && $2 =~ ^[1-9][0-9]*$ ]] || usage
		JOBS=$2
		shift 2
		;;
	--local-install)
		LOCAL_INSTALL=1
		shift
		;;
	--defer-install)
		DEFER_INSTALL=1
		shift
		;;
	--t2-config)
		T2_CONFIG=1
		shift
		;;
	--localmodconfig)
		LOCALMODCONFIG=1
		shift
		;;
	--allow-no-patches)
		ALLOW_NO_PATCHES=1
		shift
		;;
	--enable-config)
		[[ $# -ge 2 && $2 =~ ^[A-Z0-9_]+$ ]] || usage
		ENABLE_CONFIGS+=("$2")
		shift 2
		;;
	--prepare-only)
		PREPARE_ONLY=1
		shift
		;;
	--clean)
		CLEAN=1
		shift
		;;
	*) usage ;;
	esac
done

# T2 MacBook Pros with a discrete GPU use AMD graphics.  Keep this detection
# independent of loaded modules: localmodconfig may run while the dGPU is off.
if command -v lspci >/dev/null 2>&1 &&
		lspci -Dn 2>/dev/null | grep -Eqi ' (0300|0302): 1002:'; then
	HAS_AMD_DGPU=1
fi

[[ -d $PATCH_DIR ]] || {
	printf 'Patch directory does not exist: %s\n' "$PATCH_DIR" >&2
	exit 1
}
PATCH_DIR=$(cd -- "$PATCH_DIR" && pwd -P)

[[ $LOCALVERSION =~ ^[A-Za-z0-9._+-]+$ ]] || {
	printf 'Local version must be non-empty and contain only letters, numbers, dot, underscore, plus, or hyphen: %s\n' "$LOCALVERSION" >&2
	exit 2
}

for command in cpio curl find gcc git grep install make nproc patch rpm2cpio sed sha256sum sort tar uname xz yes; do
	command -v "$command" >/dev/null || {
		printf 'Missing command: %s\n' "$command" >&2
		exit 1
	}
done
if ((LOCAL_INSTALL && !DEFER_INSTALL)); then
	command -v pkexec >/dev/null || {
		printf 'Missing command: pkexec\n' >&2
		exit 1
	}
	[[ -x /usr/local/libexec/t2-kernel-builder-cleanup ]] || {
		printf 'Kernel installation helper is not installed.\n' >&2
		exit 1
	}
fi

select_kernel_release() {
	local fedora_release version release choice
	local -a versions releases candidates

	[[ -t 0 ]] || {
		printf 'KERNEL_RELEASE is required when standard input is not interactive.\n' >&2
		exit 2
	}

	# shellcheck disable=SC1091
	source /etc/os-release
	fedora_release=${VERSION_ID%%.*}

	mapfile -t versions < <(
		curl -fsSL https://kojipkgs.fedoraproject.org/packages/kernel/ |
			sed -n 's/.*href="\([0-9][^"]*\)\/".*/\1/p' |
			sort -V |
			tail -n 10
	)

	for version in "${versions[@]}"; do
		mapfile -t releases < <(
			curl -fsSL "https://kojipkgs.fedoraproject.org/packages/kernel/$version/" |
				sed -n 's/.*href="\([^"]*\.fc[0-9][0-9]*\)\/".*/\1/p' |
				sort -V
		)
		release=
		for choice in "${releases[@]}"; do
			release=$choice
		done
		[[ -n $release ]] && candidates+=("$version-$release.x86_64")
	done

	((${#candidates[@]})) || {
		printf 'No kernel builds found on Koji.\n' >&2
		exit 1
	}

	printf 'Available kernels:\n'
	for choice in "${!candidates[@]}"; do
		printf '  %d) %s\n' "$((choice + 1))" "${candidates[$choice]}"
	done
	printf 'Select kernel: '
	read -r choice
	[[ $choice =~ ^[1-9][0-9]*$ && choice -le ${#candidates[@]} ]] || {
		printf 'Invalid selection.\n' >&2
		exit 2
	}
	KERNEL_RELEASE=${candidates[$((choice - 1))]}
}
if [[ -z $KERNEL_RELEASE ]]; then
	select_kernel_release
fi

case $KERNEL_RELEASE in
*.x86_64) PACKAGE_RELEASE=${KERNEL_RELEASE%.x86_64} ;;
*)
	printf 'Expected a Fedora x86_64 release such as 7.1.7-200.fc44.x86_64\n' >&2
	exit 2
	;;
esac

VERSION=${PACKAGE_RELEASE%%-*}
RELEASE=${PACKAGE_RELEASE#*-}
[[ -n $VERSION && -n $RELEASE && $VERSION != "$RELEASE" ]] || usage
[[ $VERSION =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || {
	printf 'Expected a numeric kernel version, got: %s\n' "$VERSION" >&2
	exit 2
}
KERNEL_VERSION_MAJOR=${BASH_REMATCH[1]}
KERNEL_VERSION_PATCHLEVEL=${BASH_REMATCH[2]}
KERNEL_VERSION_SUBLEVEL=${BASH_REMATCH[3]}
KERNEL_LOCALVERSION=-${RELEASE}.x86_64${LOCALVERSION}

SRPM=kernel-$VERSION-$RELEASE.src.rpm
URL=https://kojipkgs.fedoraproject.org/packages/kernel/$VERSION/$RELEASE/src/$SRPM
WORK=$BUILD_ROOT/$KERNEL_RELEASE

if ((CLEAN)); then
	# Keep the verified SRPM and its extracted source payload. Recreate only the
	# derived patched/configured kernel tree when build inputs changed.
	rm -rf -- "$WORK/kernel"
	rm -f -- "$WORK/.prepared" "$WORK/input-hash" "$WORK/kernel-tree" \
		"$WORK/built-kernel-tree" "$WORK/built-kernel-release"
fi

mkdir -p "$WORK/download" "$WORK/sources" "$WORK/kernel"

shopt -s nullglob
PATCHES=("$PATCH_DIR"/*.patch)
shopt -u nullglob
if ((${#PATCHES[@]} == 0 && !ALLOW_NO_PATCHES)); then
	printf 'No patches found in %s\n' "$PATCH_DIR" >&2
	exit 1
fi
for patch_file in "${PATCHES[@]}"; do
	if grep -Eq '^\+config[[:space:]]+MACSMC_ACPI([[:space:]]|$)' "$patch_file"; then
		HAS_MACSMC_PATCHES=1
		break
	fi
done
if ((HAS_MACSMC_PATCHES)); then
	T2_REQUIRED_MODULES+=("${MACSMC_REQUIRED_MODULES[@]}")
fi
if ((HAS_AMD_DGPU)); then
	T2_REQUIRED_MODULES+=("${AMD_DGPU_REQUIRED_MODULES[@]}")
fi

INPUT_HASH=$({
	printf '%s\0%s\0' "$KERNEL_RELEASE" "$KERNEL_LOCALVERSION"
	printf 'build-config-schema=%s\0' "$BUILD_CONFIG_SCHEMA"
	printf 'patch-count=%s\0' "${#PATCHES[@]}"
	if ((T2_CONFIG)); then
		printf 't2-config\0'
		printf 'amd-dgpu=%s\0' "$HAS_AMD_DGPU"
		printf 'macsmc-patches=%s\0' "$HAS_MACSMC_PATCHES"
	fi
	if ((LOCALMODCONFIG)); then
		printf 'localmodconfig\0'
	fi
	printf 'enable-config=%s\0' "${ENABLE_CONFIGS[@]}"
	if ((${#PATCHES[@]})); then
		sha256sum "${PATCHES[@]}"
	fi
	if [[ -n $CONFIG_FILE ]]; then
		sha256sum "$CONFIG_FILE"
	fi
} | sha256sum | awk '{print $1}')

if [[ -f $WORK/input-hash && $(<"$WORK/input-hash") != "$INPUT_HASH" ]]; then
	printf 'Build inputs changed. Re-run with --clean.\n' >&2
	exit 1
fi

if [[ ! -s $WORK/download/$SRPM ]]; then
	printf 'Downloading %s\n' "$SRPM"
	curl --fail --location --continue-at - --output "$WORK/download/$SRPM" "$URL"
fi

if [[ ! -f $WORK/.extracted ]]; then
	printf 'Extracting Fedora source package\n'
	(
		cd "$WORK/sources"
		rpm2cpio "$WORK/download/$SRPM" | cpio -idm --quiet
	)
	touch "$WORK/.extracted"
fi

TARBALL=$(find "$WORK/sources" -maxdepth 1 -name 'linux-*.tar.xz' -print -quit)
REDHAT_PATCH=$(find "$WORK/sources" -maxdepth 1 -name 'patch-*-redhat.patch' -print -quit)
[[ -n $TARBALL && -n $REDHAT_PATCH ]] || {
	printf 'Incomplete Fedora source package\n' >&2
	exit 1
}

if [[ ! -f $WORK/.prepared ]]; then
	(
	prepare_complete=0
	cleanup_incomplete_prepare() {
		local status=$?

		if ((prepare_complete == 0)); then
			rm -rf -- "$WORK/kernel"
			rm -f -- "$WORK/.prepared" "$WORK/input-hash" "$WORK/kernel-tree"
		fi
		exit "$status"
	}
	trap cleanup_incomplete_prepare EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM

	# A failed or interrupted earlier preparation may have left an extracted,
	# partially patched tree behind. Never apply Fedora's patch set twice.
	rm -rf -- "$WORK/kernel"
	mkdir -p "$WORK/kernel"
	printf 'Preparing Fedora kernel sources\n'
	tar --no-same-owner -xf "$TARBALL" -C "$WORK/kernel"
	TREE=$(find "$WORK/kernel" -mindepth 1 -maxdepth 1 -type d -name 'linux-*' -print -quit)
	[[ -n $TREE ]]
	git -C "$TREE" init -q
	# Fedora's generated downstream patch can contain intentional trailing
	# whitespace. Keep diagnostics enabled for user-supplied patches below.
	git -C "$TREE" apply --whitespace=nowarn "$REDHAT_PATCH"
	install -m 0644 "$WORK/sources/Makefile.rhelver" "$TREE/Makefile.rhelver"
	for patch_file in "${PATCHES[@]}"; do
		printf 'Applying %s\n' "${patch_file##*/}"
		git -C "$TREE" apply "$patch_file"
	done

	# Fedora merge-window rc0 tarballs still carry the previous release in the
	# upstream Makefile.  The RPM spec rewrites PATCHLEVEL before building; do
	# the equivalent for direct local builds and keep all three fields aligned
	# with the selected Fedora package version.
	sed -i \
		-e "s/^VERSION = .*/VERSION = $KERNEL_VERSION_MAJOR/" \
		-e "s/^PATCHLEVEL = .*/PATCHLEVEL = $KERNEL_VERSION_PATCHLEVEL/" \
		-e "s/^SUBLEVEL = .*/SUBLEVEL = $KERNEL_VERSION_SUBLEVEL/" \
		"$TREE/Makefile"

	if [[ -n $CONFIG_FILE ]]; then
		install -m 0644 "$CONFIG_FILE" "$TREE/.config"
	else
		install -m 0644 "$WORK/sources/kernel-x86_64-fedora.config" "$TREE/.config"
	fi

	"$TREE/scripts/config" --file "$TREE/.config" \
		--set-str LOCALVERSION "$KERNEL_LOCALVERSION" \
		--disable LOCALVERSION_AUTO \
		--set-str SYSTEM_TRUSTED_KEYS '' \
		--set-str SYSTEM_REVOCATION_KEYS ''

	if ((T2_CONFIG)); then
		"$TREE/scripts/config" --file "$TREE/.config" --disable \
			DRM_AMDGPU --disable DRM_NOUVEAU --disable DRM_RADEON --disable DRM_XE \
			--disable CHROME_PLATFORMS --disable SURFACE_PLATFORMS \
			--disable ACER_WMI --disable ASUS_WMI --disable ASUS_NB_WMI \
			--disable DELL_LAPTOP --disable DELL_WMI \
			--disable DELL_WMI_AIO --disable DELL_WMI_DESCRIPTOR \
			--disable FUJITSU_LAPTOP --disable FUJITSU_TABLET \
			--disable GIGABYTE_WMI --disable HP_WMI \
			--disable HUAWEI_WMI --disable IDEAPAD_LAPTOP \
			--disable LG_LAPTOP --disable MSI_LAPTOP --disable MSI_WMI \
			--disable PANASONIC_LAPTOP --disable SAMSUNG_LAPTOP \
			--disable SONY_LAPTOP --disable THINKPAD_ACPI \
			--disable TOSHIBA_ACPI --disable XIAOMI_WMI \
			--disable COMPAL_LAPTOP --disable EEEPC_LAPTOP --disable EEEPC_WMI \
			--disable TOPSTAR_LAPTOP --disable PEAQ_WMI --disable MXM_WMI \
			--disable WINMATE_FM07_KEYS --disable BARCO_P50_GPIO \
			--disable PCENGINES_APU2 --disable THINKPAD_LMI \
			--disable YOGABOOK --disable YOGABOOK_WMI \
			--disable DRM_VMWGFX --disable DRM_VIRTIO_GPU --disable DRM_QXL \
			--disable DRM_BOCHS --disable DRM_CIRRUS_QEMU --disable DRM_HYPERV \
			--disable DRM_VBOXVIDEO --disable DRM_GMA500 \
			--disable DRM_MGAG200 --disable DRM_AST \
			--disable DRM_ARMADA --disable DRM_EXYNOS --disable DRM_ROCKCHIP \
			--disable DRM_MEDIATEK --disable DRM_MSM --disable DRM_TEGRA \
			--disable DRM_HISI_HIBMC --disable DRM_LOONGSON \
			--disable DRM_VC4 --disable DRM_V3D --disable DRM_ETNAVIV \
			--disable DRM_PANFROST --disable DRM_LIMA \
			--disable CPU_SUP_CENTAUR --disable CPU_SUP_ZHAOXIN --disable CPU_SUP_HYGON \
			--disable PATA_ALI --disable PATA_VIA --disable PATA_SIS \
			--disable PATA_AMD --disable PATA_ATIIXP --disable PATA_JMICRON
	fi
	if ((LOCALMODCONFIG)); then
		printf '[kait2en-progress] phase=localmodconfig\n'
		# Fedora's localmodconfig invokes oldconfig and otherwise waits forever
		# for answers when new symbols remain after module streamlining.
		set +o pipefail
		# The EFI system partition is VFAT with iocharset=iso8859-1. Its
		# modules are often unloaded while the profile is captured, and
		# without them /boot/efi fails to mount and boot ends in emergency mode.
		yes '' | make -C "$TREE" LMC_KEEP="fs/fat:fs/nls" localmodconfig
		localmod_status=${PIPESTATUS[1]}
		set -o pipefail
		((localmod_status == 0)) || exit "$localmod_status"
	fi
	if ((T2_CONFIG)); then
		# t2hid is needed for the internal keyboard/trackpad even on models
		# without a Touch Bar.  The shared t2touchbar DKMS build also links its
		# keyboard module, which requires sparse-keymap symbols from the kernel.
		# USB4 must also survive localmodconfig: pcie_ports=compat can keep the
		# in-tree Thunderbolt driver unloaded while the profile is captured.
		# t2bce_ave in the t2bce_stack DKMS build links against v4l2-mem2mem.
		# That helper has no prompt and only survives localmodconfig while
		# t2bce_ave is loaded, so VIDEO_VIM2M is kept as its selecting driver.
		# The initcall and module blacklists used by Kait2en replace drivers at
		# runtime; they do not make those drivers optional at build time.
		"$TREE/scripts/config" --file "$TREE/.config" \
			--enable INPUT_SPARSEKMAP \
			--enable HOTPLUG_PCI \
			--enable HOTPLUG_PCI_PCIE \
			--enable RTC_DRV_CMOS \
			--enable BACKLIGHT_CLASS_DEVICE \
			--enable VGA_SWITCHEROO \
			--enable MEDIA_TEST_SUPPORT \
			--enable V4L_TEST_DRIVERS
		if ((HAS_MACSMC_PATCHES)); then
			"$TREE/scripts/config" --file "$TREE/.config" --enable IIO
		fi

		if ((HAS_AMD_DGPU)); then
			# Preserve the discrete GPU stack even if localmodconfig ran while it
			# was powered off.
			"$TREE/scripts/config" --file "$TREE/.config" --module DRM_AMDGPU
		fi
	fi
	for symbol in "${ENABLE_CONFIGS[@]}"; do
		"$TREE/scripts/config" --file "$TREE/.config" --enable "$symbol"
	done
	if ((T2_CONFIG)); then
		# Keep the upstream drivers replaced by Kait2en as modules.  Their Kconfig
		# entries select infrastructure needed by the DKMS replacements, while
		# Kait2en's module_blacklist prevents the upstream modules from binding.
		# This must come after GUI overrides so none can accidentally become
		# built-in and bypass the module blacklist.
		for symbol in "${T2_REQUIRED_MODULES[@]}"; do
			"$TREE/scripts/config" --file "$TREE/.config" --module "$symbol"
		done

		# localmodconfig can promote the HDA driver to built-in. Built in, it
		# fails to probe the CS8409 codec on the MacBookPro14,1. The runtime-PM
		# installer also replaces the HDA modules with builds from this tree.
		"$TREE/scripts/config" --file "$TREE/.config" \
			--module SND_HDA_INTEL \
			--module SND_HDA_CODEC_HDMI

		if ((HAS_AMD_DGPU)); then
			# Keep i915 and apple_gmux at the same linkage level. localmodconfig
			# can promote the currently active i915 driver to built-in, but the
			# patched i915 calls apple_gmux directly and cannot link against it as
			# a module. apple_gmux must remain modular so t2gmux can replace it.
			# The runtime-PM installer also replaces the AMD modules with builds
			# from this tree. Preserve their complete Kconfig dependency graph
			# even if localmodconfig ran while the discrete GPU was off.
			"$TREE/scripts/config" --file "$TREE/.config" \
				--module DRM_AMDGPU
		fi
	fi
	make -C "$TREE" olddefconfig
	if ((T2_CONFIG)); then
		for symbol in "${T2_REQUIRED_MODULES[@]}"; do
			grep -qx "CONFIG_$symbol=m" "$TREE/.config" ||
				fail "required T2 kernel module was rejected by Kconfig: CONFIG_$symbol"
		done
		driver_symbols=(RTC_DRV_CMOS V4L2_MEM2MEM_DEV VIDEOBUF2_VMALLOC)
		if ((HAS_MACSMC_PATCHES)); then
			driver_symbols+=(IIO)
		fi
		for symbol in "${driver_symbols[@]}"; do
			grep -Eq "^CONFIG_$symbol=[ym]$" "$TREE/.config" ||
				fail "required T2 kernel driver was rejected by Kconfig: CONFIG_$symbol"
		done
	fi
	printf '%s\n' "$TREE" >"$WORK/kernel-tree"
	printf '%s\n' "$INPUT_HASH" >"$WORK/input-hash"
	touch "$WORK/.prepared"
	prepare_complete=1
	trap - EXIT INT TERM
	)
fi

TREE=$(<"$WORK/kernel-tree")

if ((T2_CONFIG)) && ! grep -Eq '^CONFIG_V4L2_MEM2MEM_DEV=[ym]$' "$TREE/.config"; then
	# Trees prepared before VIDEO_VIM2M became required can lack the
	# v4l2-mem2mem helper that t2bce_ave links against. Repair them in place
	# so an existing build only recompiles the media core.
	printf 'Enabling v4l2-mem2mem for t2bce_ave in the prepared tree\n'
	"$TREE/scripts/config" --file "$TREE/.config" \
		--enable MEDIA_TEST_SUPPORT \
		--enable V4L_TEST_DRIVERS \
		--module MEDIA_SUPPORT \
		--module VIDEO_DEV \
		--module VIDEO_VIM2M
	make -C "$TREE" olddefconfig
	for symbol in V4L2_MEM2MEM_DEV VIDEOBUF2_VMALLOC; do
		grep -Eq "^CONFIG_$symbol=[ym]$" "$TREE/.config" ||
			fail "required T2 kernel driver was rejected by Kconfig: CONFIG_$symbol"
	done
fi

if ((PREPARE_ONLY)); then
	printf 'Prepared kernel tree: %s\n' "$TREE"
	printf 'Prepared kernel release: %s\n' "$(make -s -C "$TREE" kernelrelease)"
	exit 0
fi
install -m 0644 "$WORK/sources/Makefile.rhelver" "$TREE/Makefile.rhelver"
printf 'Building %s%s with %s jobs\n' "$VERSION" "$KERNEL_LOCALVERSION" "$JOBS"

if ((LOCAL_INSTALL)); then
	BUILD_TARGETS=(bzImage modules)
	BUILD_OPTIONS=()
else
	BUILD_TARGETS=(binrpm-pkg)
	BUILD_OPTIONS=(RPMOPTS=--nodeps)
fi

if ((LOCAL_INSTALL)); then
	make -C "$TREE" -j"$JOBS" "${BUILD_TARGETS[@]}"
	KERNELRELEASE=$(make -s -C "$TREE" kernelrelease)
	if ((DEFER_INSTALL)); then
		printf '%s\n' "$TREE" >"$WORK/built-kernel-tree"
		printf '%s\n' "$KERNELRELEASE" >"$WORK/built-kernel-release"
		printf 'Built kernel tree: %s\n' "$TREE"
		printf 'Built kernel release: %s\n' "$KERNELRELEASE"
		exit 0
	fi
	printf '[kait2en-progress] phase=installing\n'
	printf 'Installing %s locally\n' "$KERNELRELEASE"
	pkexec /usr/local/libexec/t2-kernel-builder-cleanup install-kernel "$TREE" "$KERNELRELEASE"
	printf '\nInstalled kernel release: %s\n' "$KERNELRELEASE"
	exit 0
fi

make -C "$TREE" -j"$JOBS" "${BUILD_OPTIONS[@]}" "${BUILD_TARGETS[@]}"

printf '\nBuilt RPMs:\n'
find "$TREE/rpmbuild/RPMS" -type f -name '*.rpm' -print | sort
