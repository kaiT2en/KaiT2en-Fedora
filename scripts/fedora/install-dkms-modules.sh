#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dkms make install rm chown mktemp depmod

MODULES=(
	t2bce
	t2smc
	t2bdrm
	t2touchbar
	hid_t2magicmouse
	t2mfi_fastcharge
	t2gmux
	i915
	t2thunderbolt
)

DKMS_POST_TRANSACTION_OVERRIDE="/etc/dkms/framework.conf.d/kait2en-disable-post-transaction.conf"

restore_dkms_post_transaction() {
	rm -f "$DKMS_POST_TRANSACTION_OVERRIDE"
}

disable_dkms_post_transaction() {
	local tmp
	install -d -o root -g root -m 0755 /etc/dkms/framework.conf.d
	tmp="$(mktemp)"
	printf 'post_transaction=""\n' >"$tmp"
	install -o root -g root -m 0644 "$tmp" "$DKMS_POST_TRANSACTION_OVERRIDE"
	rm -f "$tmp"
	trap restore_dkms_post_transaction EXIT
}

copy_module_source() {
	local name=$1 src dst version
	src="$REPO_ROOT/modules/$name"
	[[ -f "$src/dkms.conf" ]] || fail "missing dkms.conf for $name"
	version="$(sed -n 's/^PACKAGE_VERSION="\([^"]*\)".*/\1/p' "$src/dkms.conf")"
	[[ -n "$version" ]] || fail "missing PACKAGE_VERSION in $src/dkms.conf"
	dst="/usr/src/$name-$version"

	info "copying $name source to $dst"
	rm -rf "$dst"
	install -d -o root -g root -m 0755 "$dst"
	tar -C "$src" \
		--exclude='.git' \
		--exclude='*.ko' \
		--exclude='*.o' \
		--exclude='*.mod' \
		--exclude='*.mod.c' \
		--exclude='.*.cmd' \
		--exclude='Module.symvers' \
		--exclude='modules.order' \
		-cf - . | tar -C "$dst" -xf -
	chown -R root:root "$dst"

	MODULE_VERSION="$version"
}

install_module() {
	local name=$1 version
	MODULE_VERSION=
	copy_module_source "$name"
	version="$MODULE_VERSION"

	info "registering $name/$version with DKMS"
	dkms remove --no-depmod -m "$name" -v "$version" --all >/dev/null 2>&1 || true
	dkms add -m "$name" -v "$version"
	dkms build -m "$name" -v "$version"
	dkms install --no-depmod --force -m "$name" -v "$version"
}

disable_dkms_post_transaction

for module in "${MODULES[@]}"; do
	install_module "$module"
done

depmod -a "$(kernel_release)"
restore_dkms_post_transaction
trap - EXIT

info "DKMS modules installed"
info "initramfs rebuild is handled by rebuild-initramfs.sh"
