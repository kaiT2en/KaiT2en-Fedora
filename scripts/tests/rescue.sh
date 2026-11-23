#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
script="$repo_root/auto-installer/runtime/kait2en-rescue"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-rescue-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

make_case() {
	local name=$1
	local case_dir="$work/$name"

	mkdir -p "$case_dir/bin" "$case_dir/images" "$case_dir/mnt" "$case_dir/probe"
	: >"$case_dir/mount.log"
	: >"$case_dir/umount.log"
	: >"$case_dir/chroot.log"

	cat >"$case_dir/bin/lsblk" <<EOF
#!/usr/bin/env bash
case "\$*" in
	*PATH,FSTYPE,TYPE*) cat "$case_dir/lsblk.table" ;;
	*PKNAME*) sed -n "s|^\${*: -1} ||p" "$case_dir/pkname.table" ;;
	*FSTYPE*) sed -n "s|^\${*: -1} ||p" "$case_dir/fstype.table" ;;
esac
exit 0
EOF

	cat >"$case_dir/bin/findmnt" <<EOF
#!/usr/bin/env bash
if [[ "\$*" == *--fstab* ]]; then
	fstab=
	for ((i = 1; i <= \$#; i++)); do
		[[ "\${!i}" == -F ]] || continue
		next=\$((i + 1))
		fstab=\${!next}
	done
	[[ -r "\$fstab" ]] || exit 1
	awk '\$1 !~ /^#/ && NF >= 4 { print \$2, \$1, \$3, \$4 }' "\$fstab"
	exit 0
fi
[[ -r "$case_dir/live-source" ]] || exit 1
cat "$case_dir/live-source"
EOF

	cat >"$case_dir/bin/mount" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$case_dir/mount.log"
bind=0
subvol=
args=()
while ((\$# > 0)); do
	case "\$1" in
		--bind) bind=1; shift ;;
		-t) shift 2 ;;
		-o)
			[[ "\$2" != *subvol=* ]] || subvol=\$(sed 's/.*subvol=//; s/,.*//' <<<"\$2")
			shift 2
			;;
		*) args+=("\$1"); shift ;;
	esac
done
source=\${args[0]:-}
destination=\${args[1]:-}
[[ -n "\$destination" ]] || exit 1
mkdir -p "\$destination"
((bind == 0)) || exit 0
key=\$(printf '%s' "\$source\${subvol:+@\$subvol}" | tr '/' '_')
[[ -d "$case_dir/images/\$key" ]] || exit 1
cp -a "$case_dir/images/\$key/." "\$destination/"
exit 0
EOF

	cat >"$case_dir/bin/umount" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$case_dir/umount.log"
destination=\${*: -1}
[[ "\$destination" == "$case_dir"/* ]] || exit 0
rm -rf "\${destination:?}"/* "\${destination:?}"/.[!.]* 2>/dev/null
exit 0
EOF

	cat >"$case_dir/bin/btrfs" <<EOF
#!/usr/bin/env bash
[[ "\$*" == *"subvolume list"* ]] || exit 1
[[ -r "$case_dir/subvolumes" ]] || exit 1
cat "$case_dir/subvolumes"
EOF

	cat >"$case_dir/bin/chroot" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$case_dir/chroot.log"
exit 0
EOF

	printf '%s\n' '#!/usr/bin/env bash' 'exit 1' >"$case_dir/bin/cryptsetup"
	chmod 0755 "$case_dir"/bin/*
	: >"$case_dir/lsblk.table"
	: >"$case_dir/pkname.table"
	: >"$case_dir/fstype.table"
}

make_fedora_image() {
	local case_dir=$1 key=$2
	local image="$case_dir/images/$key"

	mkdir -p "$image/etc" "$image/boot"
	printf 'ID=fedora\nVERSION_ID=44\n' >"$image/etc/os-release"
	cat >"$image/etc/fstab" <<'FSTAB'
UUID=aaaa / btrfs subvol=@,compress=zstd:1 0 0
UUID=bbbb /boot ext4 defaults 1 2
UUID=cccc /boot/efi vfat umask=0077,shortname=winnt 0 2
UUID=dddd none swap defaults 0 0
FSTAB
}

run_case() {
	local name=$1
	local case_dir="$work/$name"
	shift

	PATH="$case_dir/bin:$PATH" \
		KAIT2EN_TEST_MODE=1 \
		KAIT2EN_RESCUE_ROOT="$case_dir/mnt" \
		KAIT2EN_RESCUE_PROBE="$case_dir/probe" \
		bash "$script" "$@"
}

make_case detect
cat >"$work/detect/lsblk.table" <<'TABLE'
/dev/nvme0n1p1 vfat part
/dev/nvme0n1p2 apfs part
/dev/nvme0n1p3 ext4 part
/dev/nvme0n1p4 btrfs part
/dev/sda1 vfat part
/dev/sda2 btrfs part
TABLE
printf '/dev/sda1\n' >"$work/detect/live-source"
printf '/dev/sda1 sda\n' >"$work/detect/pkname.table"
printf '@\nhome\n' >"$work/detect/subvolumes"
mkdir -p "$work/detect/images/_dev_nvme0n1p4" "$work/detect/images/_dev_nvme0n1p3"
make_fedora_image "$work/detect" '_dev_nvme0n1p4@@'
make_fedora_image "$work/detect" '_dev_sda2'
run_case detect --list >"$work/detect.out"

grep -Fq 'Fedora 44' "$work/detect.out"
grep -Fq '/dev/nvme0n1p4' "$work/detect.out"
grep -Fq 'subvolume @' "$work/detect.out"
grep -Fq 'ignoring the live medium on /dev/sda' "$work/detect.out"
if grep -Fq '/dev/sda2' "$work/detect.out"; then
	printf 'the live medium was offered as a repair target\n' >&2
	exit 1
fi
if grep -Fq '/dev/nvme0n1p2' "$work/detect/mount.log"; then
	printf 'the macOS partition was mounted\n' >&2
	exit 1
fi

make_case nolive
cat >"$work/nolive/lsblk.table" <<'TABLE'
/dev/nvme0n1p4 btrfs part
TABLE
printf 'root\n' >"$work/nolive/subvolumes"
mkdir -p "$work/nolive/images/_dev_nvme0n1p4"
make_fedora_image "$work/nolive" '_dev_nvme0n1p4@root'
run_case nolive --list >"$work/nolive.out"
grep -Fq '/dev/nvme0n1p4' "$work/nolive.out"
grep -Fq 'subvolume root' "$work/nolive.out"

make_case luks
cat >"$work/luks/lsblk.table" <<'TABLE'
/dev/nvme0n1p4 crypto_LUKS part
TABLE
run_case luks --list </dev/null >"$work/luks.out"
grep -Fq 'is encrypted; unlocking it needs a terminal' "$work/luks.out"

make_case lvm
cat >"$work/lvm/lsblk.table" <<'TABLE'
/dev/nvme0n1p4 LVM2_member part
TABLE
run_case lvm --list >"$work/lvm.out"
grep -Fq 'LVM physical volume' "$work/lvm.out"

make_case mounting
cat >"$work/mounting/lsblk.table" <<'TABLE'
/dev/nvme0n1p4 btrfs part
TABLE
printf '@\n' >"$work/mounting/subvolumes"
mkdir -p "$work/mounting/images/_dev_nvme0n1p4" \
	"$work/mounting/images/UUID=bbbb" \
	"$work/mounting/images/UUID=cccc"
make_fedora_image "$work/mounting" '_dev_nvme0n1p4@@'
run_case mounting --shell </dev/null >"$work/mounting.out"

grep -Fq 'mounted /dev/nvme0n1p4' "$work/mounting.out"
grep -Fq 'mounted /boot from UUID=bbbb' "$work/mounting.out"
grep -Fq 'mounted /boot/efi from UUID=cccc' "$work/mounting.out"
if grep -Fq 'UUID=dddd' "$work/mounting/mount.log"; then
	printf 'the swap entry was mounted\n' >&2
	exit 1
fi
grep -Fq -- '--bind /dev ' "$work/mounting/mount.log"
grep -Fq -- '--bind /proc ' "$work/mounting/mount.log"
grep -Fq "$work/mounting/mnt /bin/bash -l" "$work/mounting/chroot.log"
grep -Fq "$work/mounting/mnt/boot/efi" "$work/mounting/umount.log"
grep -Fq "$work/mounting/mnt" "$work/mounting/umount.log"
[[ ! -d "$work/mounting/mnt" ]] ||
	[[ -z $(find "$work/mounting/mnt" -mindepth 1 -print -quit) ]]

make_case foreign
node="$work/foreign/disk"
: >"$node"
mkdir -p "$work/foreign/images/$(tr '/' '_' <<<"$node")/etc"
if run_case foreign --shell --target "$node" >/dev/null 2>&1; then
	printf 'a foreign file system was accepted as an installation\n' >&2
	exit 1
fi

printf 'Live rescue checks passed.\n'
