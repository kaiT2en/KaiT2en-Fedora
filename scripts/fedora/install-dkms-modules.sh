#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dkms make install rm chown mktemp depmod sed tar find

MODULES=(
	t2bce_dma
	t2bce_core
	t2bce_vhci
	t2bce_audio
	t2smc
	t2bdrm
	t2touchbar
	hid_t2magicmouse
	t2mfi_fastcharge
	t2gmux
	t2thunderbolt
)

LEGACY_MODULES=(
	t2dma
	t2bce
	t2bce-dma
	t2bce-core
	t2vhci
	t2audio
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

remove_dkms_module_versions() {
	local name=$1 version

	while IFS= read -r version; do
		[[ -n "$version" ]] || continue
		info "removing legacy DKMS module $name/$version"
		dkms remove --no-depmod -m "$name" -v "$version" --all >/dev/null 2>&1 || true
	done < <(dkms status -m "$name" 2>/dev/null | sed -n "s|^$name/\\([^,]*\\),.*|\\1|p")
}

remove_legacy_dkms_modules() {
	local module

	for module in "${LEGACY_MODULES[@]}"; do
		remove_dkms_module_versions "$module"
	done
}

dkms_module_version_exists() {
	local name=$1 version=$2

	dkms status -m "$name" 2>/dev/null | sed -n "s|^$name/$version\\([,:].*\\)\\?$|found|p" | grep -q '^found$'
}

purge_dkms_module_version() {
	local name=$1 version=$2 tree
	tree="/var/lib/dkms/$name/$version"

	[[ -n "$name" && -n "$version" ]] || fail "refusing to purge DKMS state with empty name or version"
	[[ "$tree" == /var/lib/dkms/*/* ]] || fail "refusing to purge unexpected DKMS path $tree"

	if [[ -e "$tree" ]]; then
		info "purging stale DKMS tree $name/$version"
		rm -rf "$tree"
	fi
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

	if [[ "$name" == "t2bce_core" ]]; then
		local t2bce_dma_version t2bce_dma_symvers

		t2bce_dma_version="$(sed -n 's/^PACKAGE_VERSION="\([^"]*\)".*/\1/p' "$REPO_ROOT/modules/t2bce_dma/dkms.conf")"
		[[ -n "$t2bce_dma_version" ]] || fail "missing PACKAGE_VERSION in $REPO_ROOT/modules/t2bce_dma/dkms.conf"
		t2bce_dma_symvers="$(find "/var/lib/dkms/t2bce_dma/$t2bce_dma_version/$(kernel_release)" -path '*/module/Module.symvers' -print -quit 2>/dev/null || true)"
		[[ -f "$t2bce_dma_symvers" ]] || fail "missing t2bce_dma Module.symvers; build t2bce_dma before t2bce_core"

		info "copying t2bce_dma interface into $dst/t2bce_dma for t2bce_core build"
		install -d -o root -g root -m 0755 "$dst/t2bce_dma"
		install -d -o root -g root -m 0755 "$dst/t2bce_dma/include"
		tar -C "$REPO_ROOT/modules/t2bce_dma/include" \
			--exclude='.git' \
			-cf - . | tar -C "$dst/t2bce_dma/include" -xf -
		install -o root -g root -m 0644 "$t2bce_dma_symvers" "$dst/t2bce_dma/Module.symvers"
	fi

	if [[ "$name" == "t2bce_audio" || "$name" == "t2bce_vhci" ]]; then
		local t2bce_core_version t2bce_core_symvers

		t2bce_core_version="$(sed -n 's/^PACKAGE_VERSION="\([^"]*\)".*/\1/p' "$REPO_ROOT/modules/t2bce_core/dkms.conf")"
		[[ -n "$t2bce_core_version" ]] || fail "missing PACKAGE_VERSION in $REPO_ROOT/modules/t2bce_core/dkms.conf"
		t2bce_core_symvers="$(find "/var/lib/dkms/t2bce_core/$t2bce_core_version/$(kernel_release)" -path '*/module/Module.symvers' -print -quit 2>/dev/null || true)"
		[[ -f "$t2bce_core_symvers" ]] || fail "missing t2bce_core Module.symvers; build t2bce_core before $name"

		info "copying t2bce_core interface into $dst/t2bce_core for $name build"
		install -d -o root -g root -m 0755 "$dst/t2bce_core"
		install -d -o root -g root -m 0755 "$dst/t2bce_core/include"
		tar -C "$REPO_ROOT/modules/t2bce_core/include" \
			--exclude='.git' \
			-cf - . | tar -C "$dst/t2bce_core/include" -xf -
		install -o root -g root -m 0644 "$t2bce_core_symvers" "$dst/t2bce_core/Module.symvers"
	fi

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
	if dkms_module_version_exists "$name" "$version"; then
		purge_dkms_module_version "$name" "$version"
	fi
	if dkms_module_version_exists "$name" "$version"; then
		fail "DKMS still contains $name/$version after purge"
	fi
	dkms add -m "$name" -v "$version"
	dkms build -m "$name" -v "$version"
	dkms install --no-depmod --force -m "$name" -v "$version"
}

disable_dkms_post_transaction
remove_legacy_dkms_modules

for module in "${MODULES[@]}"; do
	install_module "$module"
done

depmod -a "$(kernel_release)"
restore_dkms_post_transaction
trap - EXIT

info "DKMS modules installed"
info "initramfs rebuild is handled by rebuild-initramfs.sh"
