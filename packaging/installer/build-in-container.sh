#!/usr/bin/env bash

set -Eeuo pipefail

SOURCE_ROOT=${1:-/work/source}
KERNEL_DEVEL_RPM=${2:-/work/kernel-devel.rpm}
OUTPUT_DIR=${3:-/work/out}

required_variables=(
	TARGET_ID FEDORA_RELEASE ARCH DEFAULT_EDITION EDITIONS_FILE
	ISO_KERNEL_PATH ISO_INITRD_PATH ISO_KERNEL_RELEASE ANACONDA_VERSION
	KERNEL_DEVEL_SHA256 FEDORA_BASEURL INPUT_COMPAT_PATCH ARTIFACT_BASENAME
	SOURCE_DATE_EPOCH
)
for variable in "${required_variables[@]}"; do
	[[ -n ${!variable:-} ]] || {
		printf 'missing build variable: %s\n' "$variable" >&2
		exit 1
	}
done

[[ "$ARCH" =~ ^[a-z0-9_]+$ ]]
[[ "$ISO_KERNEL_RELEASE" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9]+\.fc[0-9]+\.${ARCH}$ ]]
[[ "$KERNEL_DEVEL_SHA256" =~ ^[0-9a-f]{64}$ ]]
[[ "$FEDORA_BASEURL" == https://* ]]
EDITION_CATALOG="$SOURCE_ROOT/packaging/installer/targets/$EDITIONS_FILE"
[[ -s "$EDITION_CATALOG" ]]
grep -Eq "^${DEFAULT_EDITION}[[:space:]]" "$EDITION_CATALOG"

printf '%s  %s\n' "$KERNEL_DEVEL_SHA256" "$KERNEL_DEVEL_RPM" | sha256sum --check --status

dnf install -y \
	--disablerepo='*' \
	--repofrompath="kait2en-fedora,$FEDORA_BASEURL" \
	--setopt=kait2en-fedora.gpgcheck=0 \
	--setopt=install_weak_deps=False \
	cpio \
	elfutils-libelf-devel \
	findutils \
	gcc \
	gzip \
	kmod \
	make \
	patch \
	python3 \
	rpm-build \
	tar \
	zip \
	"$KERNEL_DEVEL_RPM"

ANACONDA_VALIDATION=/work/anaconda-validation
rm -rf "$ANACONDA_VALIDATION"
mkdir -p "$ANACONDA_VALIDATION/rpm" "$ANACONDA_VALIDATION/root"
dnf download \
	--disablerepo='*' \
	--repofrompath="kait2en-fedora,$FEDORA_BASEURL" \
	--setopt=kait2en-fedora.gpgcheck=0 \
	--destdir="$ANACONDA_VALIDATION/rpm" \
	"anaconda-core-$ANACONDA_VERSION"
ANACONDA_RPM=$(find "$ANACONDA_VALIDATION/rpm" -type f -name 'anaconda-core-*.rpm' -print -quit)
[[ -n "$ANACONDA_RPM" ]]
(
	cd "$ANACONDA_VALIDATION/root"
	rpm2cpio "$ANACONDA_RPM" | cpio -idm --quiet
)

[[ -d "/usr/src/kernels/$ISO_KERNEL_RELEASE" ]]
mkdir -p "/lib/modules/$ISO_KERNEL_RELEASE"
ln -sfn "/usr/src/kernels/$ISO_KERNEL_RELEASE" "/lib/modules/$ISO_KERNEL_RELEASE/build"

BUILD_ROOT=/work/build
LAYOUT="$BUILD_ROOT/layout"
UPDATES_ROOT="$BUILD_ROOT/updates-root"
INITRAMFS_ROOT="$BUILD_ROOT/initramfs-root"

rm -rf "$BUILD_ROOT"
mkdir -p "$BUILD_ROOT/input-rpm" "$OUTPUT_DIR"
RPM_PATH=$(
	KAIT2EN_KMOD_WORK_ROOT="$BUILD_ROOT/input-kmod-work" \
		"$SOURCE_ROOT/packaging/installer/build-input-kmod.sh" \
		"$SOURCE_ROOT" \
		"$ISO_KERNEL_RELEASE" \
		"$BUILD_ROOT/input-rpm" \
		"packaging/installer/patches/$INPUT_COMPAT_PATCH" |
		tail -n 1
)
[[ -n "$RPM_PATH" && -f "$RPM_PATH" ]]

RPM_PROVIDES=$(rpm -qp --provides "$RPM_PATH")
RPM_REQUIRES=$(rpm -qp --requires "$RPM_PATH")
grep -Fx "kernel-modules = $ISO_KERNEL_RELEASE" <<<"$RPM_PROVIDES" >/dev/null
grep -Fx "kernel-core-uname-r = $ISO_KERNEL_RELEASE" <<<"$RPM_REQUIRES" >/dev/null

EXTRACT_ROOT="$BUILD_ROOT/extracted-rpm"
mkdir -p "$EXTRACT_ROOT"
(
	cd "$EXTRACT_ROOT"
	rpm2cpio "$RPM_PATH" | cpio -idm --quiet
)
RPM_FILE_LIST=$(rpm -qpl "$RPM_PATH")
for module in t2bce_dma t2bce_core t2bce_vhci t2hid hid_t2magicmouse; do
	grep -Eq "/${module}\.ko$" <<<"$RPM_FILE_LIST"
	VERMAGIC=$(modinfo -F vermagic \
		"$EXTRACT_ROOT/usr/lib/modules/$ISO_KERNEL_RELEASE/updates/kait2en/$module.ko")
	grep -Eq "^${ISO_KERNEL_RELEASE//./\\.} " <<<"$VERMAGIC"
done

mkdir -p "$LAYOUT/scripts/macos"
install -m 0644 "$SOURCE_ROOT/LICENSE" "$LAYOUT/LICENSE"
install -m 0644 \
	"$SOURCE_ROOT/packaging/installer/README.md" \
	"$LAYOUT/README.md"
install -m 0755 \
	"$SOURCE_ROOT/scripts/macos/prepare-fedora-installer.sh" \
	"$LAYOUT/scripts/macos/"
install -m 0644 "$EDITION_CATALOG" "$LAYOUT/installer-editions.tsv"

ADDON_SOURCE="$SOURCE_ROOT/packaging/installer/anaconda-addon"
ADDON_TARGET="$UPDATES_ROOT/usr/share/anaconda/addons/com_kait2en_input"
mkdir -p \
	"$ADDON_TARGET/service" \
	"$UPDATES_ROOT/usr/share/anaconda/dbus/confs" \
	"$UPDATES_ROOT/usr/share/anaconda/dbus/services" \
	"$UPDATES_ROOT/usr/share/kait2en-installer"
install -m 0644 "$ADDON_SOURCE/com_kait2en_input/__init__.py" "$ADDON_TARGET/"
install -m 0644 \
	"$ADDON_SOURCE/com_kait2en_input/service/__init__.py" \
	"$ADDON_SOURCE/com_kait2en_input/service/__main__.py" \
	"$ADDON_SOURCE/com_kait2en_input/service/installation.py" \
	"$ADDON_SOURCE/com_kait2en_input/service/kait2en.py" \
	"$ADDON_SOURCE/com_kait2en_input/service/kait2en_interface.py" \
	"$ADDON_TARGET/service/"
sed \
	-e "s|@KERNEL_RELEASE@|$ISO_KERNEL_RELEASE|g" \
	-e "s|@RPM_FILENAME@|$(basename "$RPM_PATH")|g" \
	"$ADDON_SOURCE/com_kait2en_input/service/constants.py.in" \
	>"$ADDON_TARGET/service/constants.py"
install -m 0644 \
	"$ADDON_SOURCE/org.fedoraproject.Anaconda.Addons.KaiT2en.conf" \
	"$UPDATES_ROOT/usr/share/anaconda/dbus/confs/"
install -m 0644 \
	"$ADDON_SOURCE/org.fedoraproject.Anaconda.Addons.KaiT2en.service" \
	"$UPDATES_ROOT/usr/share/anaconda/dbus/services/"
install -m 0644 "$RPM_PATH" "$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/install-wifi-firmware.sh" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/install-bt-firmware.sh" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-prepare" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-install" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-launch-terminal" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"
install -m 0644 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-install.desktop" \
	"$UPDATES_ROOT/usr/share/kait2en-installer/"

# Generated from the same tracked sources as the ISO modules. This snapshot is
# only used to bridge the first reboot; the regular installer uses GitHub main.
TRANSITION_SOURCE="$UPDATES_ROOT/usr/share/kait2en-installer/transition-source"
mkdir -p \
	"$TRANSITION_SOURCE/modules" \
	"$TRANSITION_SOURCE/packaging/installer"
install -m 0644 "$SOURCE_ROOT/LICENSE" "$TRANSITION_SOURCE/"
printf '%s\n' "$SOURCE_DATE_EPOCH" >"$TRANSITION_SOURCE/source-date-epoch"
for module in t2bce_dma t2bce_core t2bce_vhci t2touchbar hid_t2magicmouse; do
	cp -a "$SOURCE_ROOT/modules/$module" "$TRANSITION_SOURCE/modules/"
done
install -m 0755 "$SOURCE_ROOT/packaging/installer/build-input-kmod.sh" \
	"$TRANSITION_SOURCE/packaging/installer/"
install -m 0644 "$SOURCE_ROOT/packaging/installer/kmod-kait2en-input.spec" \
	"$TRANSITION_SOURCE/packaging/installer/"

while IFS= read -r python_file; do
	python3 -c \
		'import ast, pathlib, sys; ast.parse(pathlib.Path(sys.argv[1]).read_text())' \
		"$python_file"
done < <(find "$ADDON_TARGET" -type f -name '*.py' | LC_ALL=C sort)
ANACONDA_BASE=$(find "$ANACONDA_VALIDATION/root" -path '*/pyanaconda/modules/common/base/base.py' -print -quit)
ANACONDA_INTERFACE=$(find "$ANACONDA_VALIDATION/root" -path '*/pyanaconda/modules/common/base/base_interface.py' -print -quit)
[[ -n "$ANACONDA_BASE" && -n "$ANACONDA_INTERFACE" ]]
grep -Fq 'class KickstartService' "$ANACONDA_BASE"
grep -Fq 'def install_with_tasks(self):' "$ANACONDA_BASE"
grep -Fq 'def ConfigureBootloaderWithTasks' "$ANACONDA_INTERFACE"

find "$UPDATES_ROOT" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
UPDATES_IMAGE="$LAYOUT/updates.img"
(
	cd "$UPDATES_ROOT"
	find . -print0 | LC_ALL=C sort -z |
		cpio --null --create --format=newc --reproducible --owner=0:0 --quiet |
		gzip -n -9 >"$UPDATES_IMAGE"
)

MODULE_TARGET="$INITRAMFS_ROOT/usr/lib/modules/$ISO_KERNEL_RELEASE/updates/kait2en"
mkdir -p \
	"$MODULE_TARGET" \
	"$INITRAMFS_ROOT/usr/bin" \
	"$INITRAMFS_ROOT/usr/lib/kait2en" \
	"$INITRAMFS_ROOT/var/lib/dracut/hooks/pre-trigger" \
	"$INITRAMFS_ROOT/var/lib/dracut/hooks/pre-pivot"
ln -s kmod "$INITRAMFS_ROOT/usr/bin/insmod"
for module in t2bce_dma t2bce_core t2bce_vhci t2hid hid_t2magicmouse; do
	install -m 0644 \
		"$EXTRACT_ROOT/usr/lib/modules/$ISO_KERNEL_RELEASE/updates/kait2en/$module.ko" \
		"$MODULE_TARGET/"
done
# The live session installs the Apple Wi-Fi and Bluetooth firmware for itself
# from these files. The pre-pivot hook moves them into /run, which survives the
# switch root.
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/install-wifi-firmware.sh" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/install-bt-firmware.sh" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-live-wifi" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-live-bluetooth" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
install -m 0644 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-live-wifi.service" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
install -m 0644 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-live-bluetooth.service" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
# Shipped on the stick so a live session without any network can still produce
# a diagnostics archive.
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/runtime/kait2en-live-diagnostics" \
	"$INITRAMFS_ROOT/usr/lib/kait2en/"
sed "s|@KERNEL_RELEASE@|$ISO_KERNEL_RELEASE|g" \
	"$SOURCE_ROOT/packaging/installer/initramfs/20-kait2en-input.sh.in" \
	>"$INITRAMFS_ROOT/var/lib/dracut/hooks/pre-trigger/20-kait2en-input.sh"
chmod 0755 "$INITRAMFS_ROOT/var/lib/dracut/hooks/pre-trigger/20-kait2en-input.sh"
install -m 0755 \
	"$SOURCE_ROOT/packaging/installer/initramfs/90-kait2en-updates.sh" \
	"$INITRAMFS_ROOT/var/lib/dracut/hooks/pre-pivot/90-kait2en-updates.sh"
install -m 0644 "$UPDATES_IMAGE" "$INITRAMFS_ROOT/kait2en-anaconda-updates.img"
find "$INITRAMFS_ROOT" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
INITRAMFS_IMAGE="$LAYOUT/kait2en-input-initramfs.img"
(
	cd "$INITRAMFS_ROOT"
	find . -print0 | LC_ALL=C sort -z |
		cpio --null --create --format=newc --reproducible --owner=0:0 --quiet |
		gzip -n -9 >"$INITRAMFS_IMAGE"
)

sed \
	-e "s|@ISO_KERNEL_PATH@|$ISO_KERNEL_PATH|g" \
	-e "s|@ISO_INITRD_PATH@|$ISO_INITRD_PATH|g" \
	"$SOURCE_ROOT/packaging/installer/grub.cfg.in" >"$LAYOUT/grub.cfg.in"

gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/anaconda/dbus/services/org.fedoraproject.Anaconda.Addons.KaiT2en.service' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/install-wifi-firmware.sh' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/install-bt-firmware.sh' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/kait2en-install' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/kait2en-prepare' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/kait2en-launch-terminal' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/transition-source/packaging/installer/build-input-kmod.sh' >/dev/null
gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/share/kait2en-installer/transition-source/modules/t2bce_core/t2bce_main.c' >/dev/null
! gzip -dc "$UPDATES_IMAGE" | cpio -it --quiet |
	grep -F 'transition-source/packaging/installer/patches/' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx "usr/lib/modules/$ISO_KERNEL_RELEASE/updates/kait2en/t2bce_vhci.ko" >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/install-wifi-firmware.sh' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/kait2en-live-wifi' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/kait2en-live-wifi.service' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/install-bt-firmware.sh' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/kait2en-live-bluetooth' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/kait2en-live-bluetooth.service' >/dev/null
gzip -dc "$INITRAMFS_IMAGE" | cpio -it --quiet |
	grep -Fx 'usr/lib/kait2en/kait2en-live-diagnostics' >/dev/null
grep -Fq 'inst.updates=file:///run/kait2en/updates.img' "$LAYOUT/grub.cfg.in"
grep -Fq '@ISO_VOLUME_LABEL@' "$LAYOUT/grub.cfg.in"
! grep -Fq 'inst.ks=' "$LAYOUT/grub.cfg.in"
rm -f "$UPDATES_IMAGE"

printf 'TARGET_ID=%q\nFEDORA_RELEASE=%q\nDEFAULT_EDITION=%q\nISO_KERNEL_RELEASE=%q\nANACONDA_VERSION=%q\n' \
	"$TARGET_ID" "$FEDORA_RELEASE" "$DEFAULT_EDITION" \
	"$ISO_KERNEL_RELEASE" "$ANACONDA_VERSION" >"$LAYOUT/installer-target.conf"

find "$LAYOUT" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
ARTIFACT="$OUTPUT_DIR/$ARTIFACT_BASENAME.zip"
(
	cd "$LAYOUT"
	find . \( -type f -o -type l \) -print | LC_ALL=C sort | zip -X -q -y "$ARTIFACT" -@
)
(
	cd "$OUTPUT_DIR"
	sha256sum "$ARTIFACT_BASENAME.zip" >"$ARTIFACT_BASENAME.zip.sha256"
)

printf 'built %s\n' "$ARTIFACT"
