#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

APP_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$APP_DIR/../../scripts/fedora/lib.sh"

require_root
install_kait2en_fonts
require_fedora
require_command cargo make sudo

has_hybrid_t2_macbook() {
	local class dev model vendor has_intel=0 has_amd=0

	[[ -r /sys/class/dmi/id/product_name ]] || {
		info "DMI product name not found, skipping t2-dgpu-control"
		return 1
	}
	read -r model </sys/class/dmi/id/product_name
	if [[ "$model" != MacBookPro* ]]; then
		info "Model $model is not a MacBook Pro, skipping t2-dgpu-control"
		return 1
	fi

	for dev in /sys/bus/pci/devices/*; do
		[[ -r "$dev/vendor" && -r "$dev/class" ]] || continue
		read -r vendor <"$dev/vendor"
		read -r class <"$dev/class"
		[[ "$class" == 0x03* ]] || continue
		case "$vendor" in
			0x8086) has_intel=1 ;;
			0x1002) has_amd=1 ;;
		esac
	done

	if ((has_intel && has_amd)); then
		return 0
	fi
	info "Model $model has no Intel/AMD hybrid GPU layout, skipping t2-dgpu-control"
	return 1
}

if ! has_hybrid_t2_macbook; then
	exit 0
fi

target_user="${SUDO_USER:-}"
[[ -n "$target_user" && "$target_user" != root ]] ||
	fail "t2-dgpu-control must be built for the user who invoked sudo"

info "building and installing t2-dgpu-control"
build_cargo_project "$APP_DIR" "$target_user"
make -C "$APP_DIR" install

info "t2-dgpu-control installed"
