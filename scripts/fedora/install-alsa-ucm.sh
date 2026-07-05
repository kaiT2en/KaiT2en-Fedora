#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install

UCM_SRC="$REPO_ROOT/files/alsa/ucm2"
UCM_DST="/usr/share/alsa/ucm2"

[[ -r "$UCM_SRC/AppleT2/HiFi.conf" ]] ||
	fail "missing Apple T2 UCM profile in $UCM_SRC"

info "installing Apple T2 ALSA UCM profile"

install -d -o root -g root -m 0755 "$UCM_DST/AppleT2"
install -o root -g root -m 0644 "$UCM_SRC/AppleT2/HiFi.conf" "$UCM_DST/AppleT2/HiFi.conf"

for driver in AppleT2x2 AppleT2x4 AppleT2x6; do
	[[ -r "$UCM_SRC/conf.d/$driver/$driver.conf" ]] ||
		fail "missing $driver UCM profile in $UCM_SRC"

	install -d -o root -g root -m 0755 "$UCM_DST/conf.d/$driver"
	install -o root -g root -m 0644 "$UCM_SRC/conf.d/$driver/$driver.conf" "$UCM_DST/conf.d/$driver/$driver.conf"
done

info "Apple T2 ALSA UCM profile installed"
