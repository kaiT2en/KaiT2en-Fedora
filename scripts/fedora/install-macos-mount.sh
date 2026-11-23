#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command systemctl install

# The apfs module from third-party/linux-apfs-rw mounts one volume of the
# container at a time and the volume order differs between Macs and macOS
# versions. The helper therefore probes every volume of each internal APFS
# container on every boot and mounts the readable ones read-only as
# subdirectories of a single /media/macOS directory, named by the role it
# detects from the contents, so the whole macOS install is browsed from one
# place. The volume list and the encryption flags come from the container
# superblock, so the encrypted Data volume and missing volumes are never
# mounted.
info "installing Kait2en macOS volume mount"
install -d -o root -g root -m 0755 /usr/local/libexec/kait2en
tee /usr/local/libexec/kait2en/kait2en-macos-mount >/dev/null <<'EOF'
#!/bin/sh

# A single parent directory holds every readable volume as a subdirectory.
# The parent is a small tmpfs, the only mount file managers list, so the macOS
# install shows up as one entry with the volumes as folders.
PARENT=/media/macOS

case "$1" in
stop)
	for t in "$PARENT"/*; do
		[ -d "$t" ] && mountpoint -q "$t" && umount "$t"
	done
	mountpoint -q "$PARENT" && umount "$PARENT"
	exit 0
	;;
esac

modprobe apfs || exit 1

# Print the role of the volume mounted read-only at $1, or nothing. The role is
# also the subdirectory name under $PARENT.
detect_role() {
	m=$1
	if [ -f "$m/System/Library/CoreServices/SystemVersion.plist" ]; then
		echo System
	elif [ -f "$m/nvram.plist" ] && [ -d "$m/Firmware" ]; then
		echo iSCPreboot
	elif ls "$m"/*/restore/BuildManifest.plist >/dev/null 2>&1 ||
		ls -d "$m"/*/cryptex1 >/dev/null 2>&1; then
		echo Preboot
	elif ls "$m"/*/*.im4m >/dev/null 2>&1; then
		echo Recovery
	fi
}

# Print the little endian unsigned integer of $1 bytes at byte offset $2 of
# $dev.
rd() {
	od -An -tu"$1" -j "$2" -N "$1" "$dev" | tr -d ' '
}

# Print the 4 byte magic at byte offset $1 of $dev.
magic() {
	dd if="$dev" bs=1 skip="$1" count=4 status=none
}

# Print the physical block of the newest object map record for virtual object
# $1 in the object map tree rooted at block $2. Prints nothing if the object is
# not mapped.
omap_lookup() {
	oid=$1
	node=$2
	while :; do
		base=$((node * bs))
		flags=$(rd 2 $((base + 32)))
		level=$(rd 2 $((base + 34)))
		nkeys=$(rd 4 $((base + 36)))
		toff=$(rd 2 $((base + 40)))
		tlen=$(rd 2 $((base + 42)))
		keys=$((base + 56 + toff + tlen))
		vals=$((base + bs))
		# The root node keeps the 40 byte btree_info_t at its end.
		[ $((flags & 1)) -ne 0 ] && vals=$((vals - 40))
		found=
		i=0
		while [ "$i" -lt "$nkeys" ]; do
			k=$(rd 2 $((base + 56 + toff + i * 4)))
			v=$(rd 2 $((base + 58 + toff + i * 4)))
			koid=$(rd 8 $((keys + k)))
			if [ "$level" -eq 0 ]; then
				# Keys are sorted by oid and xid, the last match is the newest.
				[ "$koid" = "$oid" ] && found=$(rd 8 $((vals - v + 8)))
			else
				[ "$koid" -le "$oid" ] && found=$(rd 8 $((vals - v)))
			fi
			i=$((i + 1))
		done
		[ "$level" -eq 0 ] && break
		[ -z "$found" ] && return
		node=$found
	done
	echo "$found"
}

# Print the index of every volume of the container on $dev that the apfs
# module can read, that is every existing volume with APFS_FS_UNENCRYPTED set.
# Reads the container superblock copy in block 0, which macOS writes on every
# unmount and which the read-only mounts below never change. Mounting a
# missing or an encrypted volume makes the module log errors that look like
# corruption, so only these volumes are ever passed to mount.
readable_vols() {
	[ "$(magic 32)" = NXSB ] || return 1
	bs=$(rd 4 36)
	omap=$(rd 8 160)
	# The object map has no magic, its o_type is OBJECT_TYPE_OMAP.
	[ $(($(rd 4 $((omap * bs + 24))) & 0xffff)) -eq 11 ] || return 1
	tree=$(rd 8 $((omap * bs + 48)))
	max=$(rd 4 180)
	vol=0
	while [ "$vol" -lt "$max" ]; do
		fs=$(rd 8 $((184 + vol * 8)))
		if [ "$fs" != 0 ]; then
			blk=$(omap_lookup "$fs" "$tree")
			if [ -n "$blk" ] && [ "$(magic $((blk * bs + 32)))" = APSB ] &&
				[ $(($(rd 8 $((blk * bs + 264))) & 1)) -ne 0 ]; then
				echo "$vol"
			fi
		fi
		vol=$((vol + 1))
	done
}

probe=$(mktemp -d /run/kait2en-macos-mount.XXXXXX) || exit 1
mounted_any=0
last_err=
mkdir -p "$PARENT"
mountpoint -q "$PARENT" ||
	mount -t tmpfs -o mode=0755,size=64k,x-gvfs-show,x-gvfs-name=macOS \
		kait2en-macos "$PARENT" || exit 1

# Internal APFS partitions only. External disks are left to the file manager,
# and loop devices (mounted dmgs) are skipped so manual experiments are not
# touched.
# mount -i skips /sbin/mount.apfs. The apfs-fuse package installs that helper
# and it would take over from the kernel module and fail on these containers.
# x-gvfs-hide keeps the volumes out of the file manager sidebar, they are
# reached through the parent.
for dev in $(lsblk -lnpo PATH,FSTYPE,RM,TYPE |
	awk '$2 == "apfs" && $3 == "0" && $4 == "part" { print $1 }'); do
	if ! vols=$(readable_vols); then
		last_err="$dev: no valid APFS container superblock"
		continue
	fi
	for vol in $vols; do
		if err=$(mount -i -t apfs -o ro,vol="$vol" "$dev" "$probe" 2>&1); then
			role=$(detect_role "$probe")
			umount "$probe"
			if [ -n "$role" ] && ! mountpoint -q "$PARENT/$role"; then
				mkdir -p "$PARENT/$role"
				if mount -i -t apfs -o ro,vol="$vol",x-gvfs-hide \
					"$dev" "$PARENT/$role"; then
					mounted_any=1
				fi
			fi
		else
			last_err="$dev vol=$vol: $err"
		fi
	done
done

rmdir "$probe"
if [ "$mounted_any" != 1 ]; then
	echo "no readable macOS volume found" >&2
	[ -n "$last_err" ] && echo "last error: $last_err" >&2
fi
exit 0
EOF
chmod 0755 /usr/local/libexec/kait2en/kait2en-macos-mount

tee /etc/systemd/system/kait2en-macos-mount.service >/dev/null <<'EOF'
[Unit]
Description=Kait2en mount the readable macOS volumes read-only
After=local-fs.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/local/libexec/kait2en/kait2en-macos-mount
ExecStop=/usr/local/libexec/kait2en/kait2en-macos-mount stop

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 /etc/systemd/system/kait2en-macos-mount.service

# File managers would list the partition itself as a mountable entry, and a
# click mounts the encrypted Data volume. The volumes are reached through
# /media/macOS, so the partition is hidden.
tee /etc/udev/rules.d/90-kait2en-macos.rules >/dev/null <<'EOF'
SUBSYSTEM=="block", ENV{DEVTYPE}=="partition", KERNEL=="nvme*", ENV{ID_FS_TYPE}=="apfs", ENV{UDISKS_IGNORE}="1"
EOF
chmod 0644 /etc/udev/rules.d/90-kait2en-macos.rules
udevadm control --reload
udevadm trigger --action=change --subsystem-match=block --property-match=ID_FS_TYPE=apfs

systemctl daemon-reload
systemctl enable kait2en-macos-mount.service

info "Kait2en macOS volume mount installed"
