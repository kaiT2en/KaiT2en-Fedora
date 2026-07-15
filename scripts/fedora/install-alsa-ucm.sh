#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command basename cp cut getent id install readlink rm runuser sed systemctl

UCM_SRC="$REPO_ROOT/modules/t2bce_audio-alsa-ucm-conf/ucm2"
UCM_DST="/usr/share/alsa/ucm2"

target_user_home() {
	local user=${SUDO_USER:-}

	if [[ -n "$user" && "$user" != root ]]; then
		getent passwd "$user" | cut -d: -f6
	else
		printf '%s\n' "${HOME:-/root}"
	fi
}

target_user() {
	local user=${SUDO_USER:-}

	if [[ -n "$user" && "$user" != root ]]; then
		printf '%s\n' "$user"
	else
		return 1
	fi
}

restart_user_audio() {
	local user uid runtime

	if ! user="$(target_user)"; then
		warn "cannot determine non-root user; restart WirePlumber and PipeWire manually"
		return 0
	fi

	uid="$(id -u "$user")"
	runtime="/run/user/$uid"
	if [[ ! -d "$runtime" ]]; then
		warn "user runtime $runtime is not active; restart WirePlumber and PipeWire after login"
		return 0
	fi

	info "restarting WirePlumber and PipeWire for $user"
	if ! runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
		systemctl --user restart wireplumber pipewire pipewire-pulse; then
		warn "could not restart user audio services; reboot or restart them manually"
	fi
}

apple_t2_wireplumber_devices() {
	local card dev_path pci_path vendor device pci_id pci_tag

	for card in /sys/class/sound/card*; do
		[[ -e "$card" ]] || continue

		dev_path="$(readlink -f "$card/device" 2>/dev/null || true)"
		pci_path="$dev_path"

		while [[ -n "$pci_path" && "$pci_path" != / ]]; do
			if [[ -r "$pci_path/vendor" && -r "$pci_path/device" ]]; then
				vendor="$(<"$pci_path/vendor")"
				device="$(<"$pci_path/device")"
				if [[ "$vendor" == "0x106b" && "$device" == "0x1803" ]]; then
					pci_id="$(basename "$pci_path")"
					pci_tag="${pci_id//:/_}"
					printf 'alsa_card.pci-%s\n' "$pci_tag"
				fi
				break
			fi
			pci_path="${pci_path%/*}"
		done
	done
}

reset_wireplumber_t2_profile_state() {
	local home state_dir default_profile default_nodes device escaped changed=

	home="$(target_user_home)"
	state_dir="$home/.local/state/wireplumber"
	default_profile="$state_dir/default-profile"
	default_nodes="$state_dir/default-nodes"

	[[ -d "$state_dir" ]] || return 0

	while IFS= read -r device; do
		escaped="${device//./\\.}"

		if [[ -f "$default_profile" ]]; then
			[[ -e "$default_profile.kait2en.bak" ]] || cp -p "$default_profile" "$default_profile.kait2en.bak"
			sed -i "/^${escaped}=/d" "$default_profile"
			changed=1
		fi

		if [[ -f "$default_nodes" ]]; then
			[[ -e "$default_nodes.kait2en.bak" ]] || cp -p "$default_nodes" "$default_nodes.kait2en.bak"
			sed -i "/${escaped}/d" "$default_nodes"
			changed=1
		fi
	done < <(apple_t2_wireplumber_devices)

	if [[ -n "$changed" ]]; then
		info "reset stored WirePlumber profile defaults for Apple T2 audio"
	fi
}

info "installing Apple T2 ALSA UCM profile"

for profile in HiFi-x2 HiFi-x4 HiFi-x6; do
	[[ -r "$UCM_SRC/AppleT2/$profile.conf" ]] ||
		fail "missing $profile UCM profile in $UCM_SRC"
done

for driver in AppleT2x2 AppleT2x4 AppleT2x6; do
	[[ -r "$UCM_SRC/conf.d/$driver/$driver.conf" ]] ||
		fail "missing $driver UCM profile in $UCM_SRC"
done

install -d -o root -g root -m 0755 "$UCM_DST/AppleT2"

# Remove KaiT2en profile files superseded by the split UCM layout. Keep this
# list explicit: /usr/share/alsa/ucm2 may also contain distribution files.
rm -f \
	"$UCM_DST/AppleT2/HiFi.conf" \
	"$UCM_DST/AppleT2/Measurement-x4.conf"

for profile in HiFi-x2 HiFi-x4 HiFi-x6; do
	install -o root -g root -m 0644 \
		"$UCM_SRC/AppleT2/$profile.conf" "$UCM_DST/AppleT2/$profile.conf"
done

for driver in AppleT2x2 AppleT2x4 AppleT2x6; do
	install -d -o root -g root -m 0755 "$UCM_DST/conf.d/$driver"
	install -o root -g root -m 0644 "$UCM_SRC/conf.d/$driver/$driver.conf" "$UCM_DST/conf.d/$driver/$driver.conf"
done

reset_wireplumber_t2_profile_state
restart_user_audio

info "Apple T2 ALSA UCM profile installed"
