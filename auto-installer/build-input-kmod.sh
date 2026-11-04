#!/usr/bin/env bash

set -Eeuo pipefail

SOURCE_ROOT=${1:?source root is required}
KERNEL_RELEASE=${2:?kernel release is required}
OUTPUT_DIR=${3:?output directory is required}

[[ "$KERNEL_RELEASE" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9]+\.fc[0-9]+\.([a-z0-9_]+)$ ]] || {
	printf 'invalid Fedora kernel release: %s\n' "$KERNEL_RELEASE" >&2
	exit 1
}
ARCH=${BASH_REMATCH[1]}
if [[ -z ${SOURCE_DATE_EPOCH:-} ]]; then
	command -v git >/dev/null 2>&1 || {
		printf 'missing input-kmod build command: git\n' >&2
		exit 1
	}
	SOURCE_DATE_EPOCH=$(git -C "$SOURCE_ROOT" log -1 --format=%ct)
fi
[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]

for command in make rpmbuild tar; do
	command -v "$command" >/dev/null 2>&1 || {
		printf 'missing input-kmod build command: %s\n' "$command" >&2
		exit 1
	}
done
if [[ -n ${KAIT2EN_KMOD_WORK_ROOT:-} ]]; then
	work=$KAIT2EN_KMOD_WORK_ROOT
	rm -rf "$work"
	mkdir -p "$work"
else
	work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-input-kmod.XXXXXX")
fi
cleanup() {
	rm -rf "$work"
}
trap cleanup EXIT

rpm_root="$work/rpmbuild"
source_stage="$work/kait2en-input-0.1"
mkdir -p "$rpm_root"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS} \
	"$source_stage/modules" "$OUTPUT_DIR"

install -m 0644 "$SOURCE_ROOT/LICENSE" "$source_stage/LICENSE"
for module in t2bce_dma t2bce_core t2bce_vhci t2touchbar hid_t2magicmouse t2_precision_trackpad t2_trackpad_actuator; do
	cp -a "$SOURCE_ROOT/modules/$module" "$source_stage/modules/$module"
done

tar --sort=name \
	--mtime="@$SOURCE_DATE_EPOCH" \
	--owner=0 --group=0 --numeric-owner \
	-C "$work" -czf "$rpm_root/SOURCES/kait2en-input-0.1.tar.gz" \
	kait2en-input-0.1
install -m 0644 \
	"$SOURCE_ROOT/auto-installer/kmod-kait2en-input.spec" \
	"$rpm_root/SPECS/"

export SOURCE_DATE_EPOCH
rpmbuild -bb \
	--target "$ARCH" \
	--define "_topdir $rpm_root" \
	--define "_buildhost kait2en-installer.invalid" \
	--define "_buildtime $SOURCE_DATE_EPOCH" \
	--define "build_mtime_policy clamp_to_source_date_epoch" \
	--define "kernel_release $KERNEL_RELEASE" \
	"$rpm_root/SPECS/kmod-kait2en-input.spec"

rpm_path=$(find "$rpm_root/RPMS/$ARCH" -maxdepth 1 -type f \
	-name 'kmod-kait2en-input-*.rpm' -print -quit)
[[ -n "$rpm_path" && -f "$rpm_path" ]] || {
	printf 'input-kmod RPM was not produced for %s\n' "$KERNEL_RELEASE" >&2
	exit 1
}
install -m 0644 "$rpm_path" "$OUTPUT_DIR/"
printf '%s\n' "$OUTPUT_DIR/${rpm_path##*/}"
