#!/usr/bin/env bash

APP_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$APP_DIR/../../scripts/fedora/lib.sh"

require_root
require_fedora
require_command cargo make sudo

is_supported_model() {
	local class dev model vendor has_intel=0 has_amd=0

	[[ -r /sys/class/dmi/id/product_name ]] || {
		info "DMI product name not found, skipping t2-hybrid-gpu-control"
		return 1
	}
	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBookPro15,1|MacBookPro15,3|MacBookPro16,1|MacBookPro16,4) ;;
		*)
			info "GPU runtime PM is not supported on $model, skipping t2-hybrid-gpu-control"
			return 1
			;;
	esac

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
	info "Model $model has no Intel/AMD hybrid GPU layout, skipping t2-hybrid-gpu-control"
	return 1
}

if ! is_supported_model; then
	exit 0
fi

target_user="${SUDO_USER:-}"
[[ -n "$target_user" && "$target_user" != root ]] ||
	fail "t2-hybrid-gpu-control must be built for the user who invoked sudo"

info "building and installing t2-hybrid-gpu-control"
sudo -H -u "$target_user" make -C "$APP_DIR" clean
sudo -H -u "$target_user" make -C "$APP_DIR" build
make -C "$APP_DIR" install

info "t2-hybrid-gpu-control installed"
