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
for asset in kait2en.plymouth kait2en.script \
		boot.png bullet.png entry.png keyboard.png lock.png progress_bar.png progress_box.png; do
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

# Plymouth's script plugin does not expose the active keyboard layout. Resolve
# the configured XKB layout and variant now, then bake its display name into
# the splash for the LUKS prompt.
keyboard_layout_id=us
keyboard_layout_variant=
if [[ -r /etc/vconsole.conf ]]; then
	while IFS='=' read -r key value; do
		value=${value#\"}
		value=${value%\"}
		case $key in
			XKBLAYOUT)
				value=${value%%,*}
				[[ $value =~ ^[[:alnum:]_-]+$ ]] && keyboard_layout_id=$value
				;;
			XKBVARIANT)
				value=${value%%,*}
				[[ $value =~ ^[[:alnum:]_-]*$ ]] && keyboard_layout_variant=$value
				;;
		esac
	done </etc/vconsole.conf
fi
if command -v python3 >/dev/null 2>&1; then
	python3 - "$THEME_DST/kait2en.script" "$keyboard_layout_id" \
		"$keyboard_layout_variant" /usr/share/X11/xkb/rules/evdev.xml <<'PY'
import json
import pathlib
import sys
import xml.etree.ElementTree as ET

script_path = pathlib.Path(sys.argv[1])
layout_id, variant_id, rules_path = sys.argv[2:]
label = layout_id

try:
	root = ET.parse(rules_path).getroot()
	for layout in root.findall('./layoutList/layout'):
		config = layout.find('configItem')
		if config is None or config.findtext('name') != layout_id:
			continue
		label = config.findtext('description') or layout_id
		for variant in layout.findall('./variantList/variant'):
			variant_config = variant.find('configItem')
			if variant_config is not None and variant_config.findtext('name') == variant_id:
				label = variant_config.findtext('description') or label
				break
		break
except (OSError, ET.ParseError):
	pass

source = script_path.read_text()
source = source.replace(
	'keyboard_layout_label = "English (US)";',
	'keyboard_layout_label = ' + json.dumps(label, ensure_ascii=False) + ';',
)
script_path.write_text(source)
PY
else
	sed -i "s/^keyboard_layout_label = .*/keyboard_layout_label = \"$keyboard_layout_id\";/" \
		"$THEME_DST/kait2en.script"
fi

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
