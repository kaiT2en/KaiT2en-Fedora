#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dkms make install rm chown mktemp depmod sed tar find grep
require_kernel_headers

MODULES=(
	t2bce_stack
	t2smc
	t2bdrm
	t2touchbar
	hid_t2magicmouse
	t2mfi_fastcharge
	t2gmux
	t2thunderbolt
	t2smp
)

LEGACY_MODULES=(
	t2sep
	t2bce_dma
	t2bce_core
	t2bce_vhci
	t2bce_audio
	t2bce
	t2dma
	t2audio
	t2vhci
	t2bce-core
	t2bce-dma
)

RETIRED_BCE_DKMS_PACKAGES=(
	t2bce_dma
	t2bce_core
	t2bce_vhci
	t2bce_audio
)

DKMS_POST_TRANSACTION_OVERRIDE="/etc/dkms/framework.conf.d/kait2en-disable-post-transaction.conf"
DKMS_KERNEL_INSTALL_HOOK="/etc/kernel/install.d/39-kait2en-dkms-cleanup.install"

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

install_dkms_kernel_hook() {
	local tmp

	install -d -o root -g root -m 0755 /etc/kernel/install.d
	tmp="$(mktemp)"
	cat >"$tmp" <<'HOOK'
#!/usr/bin/bash

set -u

command=${1:-}
kernelver=${2:-}

[[ "$command" == add && -n "$kernelver" ]] || exit 0

modules=(
	t2bce_stack
	t2smc
	t2bdrm
	t2touchbar
	hid_t2magicmouse
	t2mfi_fastcharge
	t2gmux
	t2thunderbolt
	t2smp
)

for name in "${modules[@]}"; do
	for conf in /usr/src/"$name"-*/dkms.conf; do
		[[ -f "$conf" ]] || continue
		version="$(sed -n 's/^PACKAGE_VERSION="\([^"]*\)".*/\1/p' "$conf")"
		[[ -n "$version" ]] || continue

		if dkms status -m "$name" -v "$version" -k "$kernelver" 2>/dev/null |
				grep -Fq ': installed'; then
			continue
		fi

		tree="/var/lib/dkms/$name/$version/$kernelver"
		[[ "$tree" == /var/lib/dkms/*/*/* && -d "$tree" ]] || continue

		printf '[kait2en] removing stale DKMS build tree %s/%s for %s\n' \
			"$name" "$version" "$kernelver"
		dkms remove --no-depmod -m "$name" -v "$version" -k "$kernelver" \
			>/dev/null 2>&1 || rm -rf "$tree"
	done
done

exit 0
HOOK
	install -o root -g root -m 0755 "$tmp" "$DKMS_KERNEL_INSTALL_HOOK"
	rm -f "$tmp"
}

remove_dkms_module_versions_for_kernel() {
	local name=$1 kernel version
	local -A seen=()
	kernel="$(kernel_release)"

	while IFS= read -r version; do
		[[ -n "$version" ]] || continue
		[[ -z "${seen[$version]:-}" ]] || continue
		seen[$version]=1
		dkms status -m "$name" -v "$version" -k "$kernel" 2>/dev/null |
			grep -q . || continue
		info "removing DKMS module $name/$version for $kernel"
		dkms remove --no-depmod -m "$name" -v "$version" -k "$kernel"
	done < <(dkms status -m "$name" 2>/dev/null | sed -n "s|^$name/\\([^,:]*\\)[,:].*|\\1|p")
}

remove_legacy_dkms_modules() {
	local module conf

	for module in "${LEGACY_MODULES[@]}"; do
		remove_dkms_module_versions_for_kernel "$module"
	done

	# Older kernels may still rely on the split packages. Keep those installed,
	# but stop DKMS from automatically rebuilding them alongside t2bce_stack for
	# future kernels. They can be removed once those older kernels are retired.
	for module in "${RETIRED_BCE_DKMS_PACKAGES[@]}"; do
		for conf in /usr/src/"$module"-*/dkms.conf; do
			[[ -f "$conf" ]] || continue
			info "disabling autoinstall for retired DKMS package ${conf%/dkms.conf}"
			if grep -q '^AUTOINSTALL=' "$conf"; then
				sed -i 's/^AUTOINSTALL=.*/AUTOINSTALL="no"/' "$conf"
			else
				printf 'AUTOINSTALL="no"\n' >>"$conf"
			fi
		done
	done
}

remove_repo_dkms_modules() {
	local module

	for module in "${MODULES[@]}"; do
		remove_dkms_module_versions_for_kernel "$module"
	done
}

migrate_keyboard_backlight_state() {
	local old_state new_state
	local old_suffix=':leds:apple::kbd_backlight'
	local new_suffix=':leds::white:kbd_backlight'

	[[ -d /var/lib/systemd/backlight ]] || return 0

	while IFS= read -r -d '' old_state; do
		new_state="${old_state%"$old_suffix"}$new_suffix"
		[[ "$new_state" != "$old_state" ]] ||
			fail "refusing to migrate unexpected backlight state path $old_state"
		[[ -e "$new_state" ]] && continue

		info "migrating keyboard backlight state to the standard LED name"
		install -m 0644 "$old_state" "$new_state"
		chown --reference="$old_state" "$new_state"
	done < <(find /var/lib/systemd/backlight -maxdepth 1 -type f \
		-name "*$old_suffix" -print0)
}

dkms_module_version_exists() {
	local name=$1 version=$2

	dkms status -m "$name" 2>/dev/null | sed -n "s|^$name/$version\\([,:].*\\)\\?$|found|p" | grep -q '^found$'
}

dkms_module_version_installed() {
	local name=$1 version=$2 kernel
	kernel="$(kernel_release)"

	dkms status -m "$name" -v "$version" -k "$kernel" 2>/dev/null | grep -Fq ': installed'
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

	if [[ "$name" == "t2bce_stack" ]]; then
		local component
		for component in t2bce_dma t2bce_core t2bce_vhci t2bce_audio t2bce_ave; do
			info "staging $component in $dst"
			install -d -o root -g root -m 0755 "$dst/$component"
			tar -C "$REPO_ROOT/modules/$component" \
				--exclude='.git' \
				--exclude='*.ko' \
				--exclude='*.o' \
				--exclude='*.mod' \
				--exclude='*.mod.c' \
				--exclude='.*.cmd' \
				--exclude='Module.symvers' \
				--exclude='modules.order' \
				-cf - . | tar -C "$dst/$component" -xf -
		done
	fi

	chown -R root:root "$dst"

	MODULE_VERSION="$version"
}

install_module() {
	local name=$1 version kernel
	MODULE_VERSION=
	copy_module_source "$name"
	version="$MODULE_VERSION"
	kernel="$(kernel_release)"

	if ! dkms_module_version_exists "$name" "$version"; then
		info "registering $name/$version with DKMS"
		dkms add -m "$name" -v "$version"
	fi
	dkms build -m "$name" -v "$version" -k "$kernel"
	dkms install --no-depmod --force -m "$name" -v "$version" -k "$kernel"
	if ! dkms_module_version_installed "$name" "$version"; then
		fail "DKMS did not install $name/$version for kernel $(kernel_release)"
	fi
}

install_dkms_kernel_hook
disable_dkms_post_transaction
migrate_keyboard_backlight_state
remove_legacy_dkms_modules
remove_repo_dkms_modules

for module in "${MODULES[@]}"; do
	install_module "$module"
done

depmod -a "$(kernel_release)"
restore_dkms_post_transaction
trap - EXIT

info "DKMS modules installed"
info "initramfs rebuild is handled by rebuild-initramfs.sh"
