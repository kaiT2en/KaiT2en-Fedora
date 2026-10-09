#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora

defer_initramfs=false
if [[ ${1:-} == --defer-initramfs ]]; then
	defer_initramfs=true
	shift
fi
(( $# == 0 )) || fail "usage: $0 [--defer-initramfs]"

THEME_SRC="$REPO_ROOT/assets/plymouth/kait2en"
THEME_DST="/usr/share/plymouth/themes/kait2en"
PLYMOUTHD_CONF="/etc/plymouth/plymouthd.conf"

# The EFI framebuffer runs at 2880x1800 while the panel runs at 2560x1600, so
# Fedora's scale heuristic picks 2 on simpledrm and 1 once i915 takes over,
# and the i915 takeover briefly shows the simpledrm buffer unscaled. Skip
# simpledrm and pin the scale; the theme sizes itself from the window size.
set_daemon_option() {
	local key=$1 value=$2

	sed -i -e "/^${key}[[:blank:]]*=.*/d" "$PLYMOUTHD_CONF"
	sed -i -e "s/^\[Daemon\]\$/[Daemon]\n${key}=${value}/" "$PLYMOUTHD_CONF"
}

missing_requirements=()
for command_name in grep install plymouth-set-default-theme rm sed; do
	command -v "$command_name" >/dev/null 2>&1 ||
		missing_requirements+=("command $command_name")
done
for asset in kait2en.plymouth kait2en.script watermark.png \
		boot.png box.png bullet.png entry.png lock.png progress_bar.png progress_box.png; do
	[[ -r "$THEME_SRC/$asset" ]] || missing_requirements+=("asset $asset")
done

if (( ${#missing_requirements[@]} > 0 )); then
	warn "skipping KaiT2en Plymouth theme; missing: ${missing_requirements[*]}"
	exit 0
fi

info "installing KaiT2en Plymouth theme"

rm -rf "$THEME_DST"
install -d -o root -g root -m 0755 "$THEME_DST"
install -o root -g root -m 0644 \
	"$THEME_SRC"/*.png \
	"$THEME_SRC/kait2en.plymouth" \
	"$THEME_SRC/kait2en.script" \
	"$THEME_DST/"

install -d -o root -g root -m 0755 "$(dirname "$PLYMOUTHD_CONF")"
touch "$PLYMOUTHD_CONF"
grep -q '^\[Daemon\]$' "$PLYMOUTHD_CONF" || echo '[Daemon]' >>"$PLYMOUTHD_CONF"
set_daemon_option DeviceScale 1
set_daemon_option UseSimpledrm 0

if "$defer_initramfs"; then
	plymouth-set-default-theme kait2en
else
	plymouth-set-default-theme -R kait2en
fi

info "KaiT2en Plymouth theme installed"
