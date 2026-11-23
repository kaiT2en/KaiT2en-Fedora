#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_repo_root
require_fedora

[[ ${EUID:-$(id -u)} -ne 0 ]] || fail "run this script as your user, not with sudo"

remove=false
if [[ ${1:-} == --remove ]]; then
	remove=true
	shift
fi
(( $# == 0 )) || fail "usage: $0 [--remove]"

THEME_SRC="$REPO_ROOT/assets/vornehm"
GTK4_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/gtk-4.0"
GTK4_CSS="$GTK4_DIR/gtk.css"
GTK4_BACKUP="$GTK4_CSS.kait2en-vornehm-backup"
IMPORT_LINE='@import url("kait2en-vornehm.css");'
EXT_UUID="kait2en-vornehm@kait2en"
EXT_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$EXT_UUID"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/kait2en-vornehm"
ICON_PREVIOUS="$ICON_DIR/previous-icon-theme"
ADWAITA_PLACES=/usr/share/icons/Adwaita/scalable/places
BG_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/backgrounds/kait2en-vornehm"
BG_PROPS="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-background-properties/kait2en-vornehm.xml"
BG_DEFAULT=black-viii.jpg
BG_DEFAULT_LIGHT=white-viii.jpg
SETTINGS_PREVIOUS="$BG_DIR/previous-settings"
PTYXIS_PALETTES="${XDG_DATA_HOME:-$HOME/.local/share}/org.gnome.Ptyxis/palettes"
PTYXIS_PALETTE=kait2en-vornehm
LOOK_KEYS=(
	"org.gnome.desktop.background picture-uri"
	"org.gnome.desktop.background picture-uri-dark"
	"org.gnome.desktop.background picture-options"
	"org.gnome.desktop.interface color-scheme"
)

# The shell only notices a new extension after the next login, so the
# enabled list is edited directly when gnome-extensions does not know it yet.
set_extension_enabled() {
	local enabled
	enabled=$(gsettings get org.gnome.shell enabled-extensions)
	if [[ $1 == true ]]; then
		gnome-extensions enable "$EXT_UUID" 2>/dev/null && return 0
		[[ $enabled == *"'$EXT_UUID'"* ]] && return 0
		if [[ $enabled == "@as []" || $enabled == "[]" ]]; then
			enabled="['$EXT_UUID']"
		else
			enabled="${enabled%]}, '$EXT_UUID']"
		fi
	else
		gnome-extensions disable "$EXT_UUID" 2>/dev/null || true
		enabled=$(gsettings get org.gnome.shell enabled-extensions)
		enabled=${enabled//", '$EXT_UUID'"/}
		enabled=${enabled//"'$EXT_UUID', "/}
		enabled=${enabled//"'$EXT_UUID'"/}
	fi
	gsettings set org.gnome.shell enabled-extensions "$enabled"
}

# Adwaita folders recolored to graphite. They are generated from the
# installed Adwaita, so they follow its updates and nothing is redistributed.
build_icon_theme() {
	local svg
	rm -rf "$ICON_DIR/scalable"
	install -d -m 0755 "$ICON_DIR/scalable/places"
	for svg in "$ADWAITA_PLACES"/*.svg; do
		grep -q '#438de6' "$svg" || continue
		sed -e '0,/#438de6/s//#38383e/' -e 's/#438de6/#b0b0b8/g' \
			-e 's/#62a0ea/#4c4c53/g' -e 's/#afd4ff/#74747c/g' \
			-e 's/#c0d5ea/#68686f/g' -e 's/#a4caee/#606068/g' \
			"$svg" >"$ICON_DIR/scalable/places/${svg##*/}"
	done
	cat >"$ICON_DIR/index.theme" <<-'EOF'
		[Icon Theme]
		Name=KaiT2en Vornehm
		Comment=Adwaita with graphite folders
		Inherits=Adwaita,hicolor
		Directories=scalable/places

		[scalable/places]
		Context=Places
		Size=128
		MinSize=8
		MaxSize=512
		Type=Scalable
	EOF
}

# The BLACK wallpapers by Jean-Marc Denis, listed in the GNOME settings.
# Their credits are installed next to them. A BLACK image with a WHITE
# image of the same number is one entry, GNOME shows the WHITE image in
# the default style and the BLACK image in the dark style.
install_backgrounds() {
	local image name light
	install -d -m 0755 "$BG_DIR" "${BG_PROPS%/*}"
	install -m 0644 "$THEME_SRC/backgrounds"/*.jpg "$THEME_SRC/backgrounds/CREDITS.md" "$BG_DIR/"
	{
		printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' \
			'<!DOCTYPE wallpapers SYSTEM "gnome-wp-list.dtd">' '<wallpapers>'
		for image in "$BG_DIR"/black-*.jpg; do
			name=${image##*/}
			name=${name%.jpg}
			name=${name#black-}
			light="$BG_DIR/white-$name.jpg"
			printf '  <wallpaper deleted="false">\n'
			if [ -f "$light" ]; then
				printf '    <name>WHITE and BLACK %s, Jean-Marc Denis</name>\n' "${name^^}"
				printf '    <filename>%s</filename>\n' "$light"
				printf '    <filename-dark>%s</filename-dark>\n' "$image"
			else
				printf '    <name>BLACK %s, Jean-Marc Denis</name>\n' "${name^^}"
				printf '    <filename>%s</filename>\n' "$image"
			fi
			printf '    <options>zoom</options>\n'
			printf '    <shade_type>solid</shade_type>\n'
			printf '    <pcolor>#000000</pcolor>\n'
			printf '    <scolor>#000000</scolor>\n'
			printf '  </wallpaper>\n'
		done
		printf '%s\n' '</wallpapers>'
	} >"$BG_PROPS"
}

# Each setting is saved once before its first change, so a later run keeps
# the value from before KaiT2en Vornehm. Settings added in a newer version
# are appended on the next run.
save_look_settings() {
	local key
	touch "$SETTINGS_PREVIOUS"
	for key in "${LOOK_KEYS[@]}"; do
		grep -q "^$key " "$SETTINGS_PREVIOUS" && continue
		printf '%s %s\n' "$key" "$(gsettings get $key)" >>"$SETTINGS_PREVIOUS"
	done
}

# Ptyxis, the Fedora terminal, gets the KaiT2en Vornehm palette in its
# default profile. On a fresh system that profile does not exist before the
# first start, so one is created.
PTYXIS_PROFILE=
prepare_ptyxis() {
	gsettings list-keys org.gnome.Ptyxis >/dev/null 2>&1 || return 0
	PTYXIS_PROFILE=$(gsettings get org.gnome.Ptyxis default-profile-uuid)
	PTYXIS_PROFILE=${PTYXIS_PROFILE//\'/}
	[[ -n $PTYXIS_PROFILE ]] || PTYXIS_PROFILE=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
	LOOK_KEYS+=(
		"org.gnome.Ptyxis profile-uuids"
		"org.gnome.Ptyxis default-profile-uuid"
		"org.gnome.Ptyxis font-name"
		"org.gnome.Ptyxis use-system-font"
		"org.gnome.Ptyxis.Profile:/org/gnome/Ptyxis/Profiles/$PTYXIS_PROFILE/ palette"
	)
}

apply_ptyxis() {
	local profiles font=Monospace
	[[ -n $PTYXIS_PROFILE ]] || return 0
	install -d -m 0755 "$PTYXIS_PALETTES"
	install -m 0644 "$THEME_SRC/ptyxis/$PTYXIS_PALETTE.palette" "$PTYXIS_PALETTES/"
	profiles=$(gsettings get org.gnome.Ptyxis profile-uuids)
	if [[ $profiles != *"'$PTYXIS_PROFILE'"* ]]; then
		gsettings set org.gnome.Ptyxis profile-uuids "['$PTYXIS_PROFILE']"
		gsettings set org.gnome.Ptyxis default-profile-uuid "$PTYXIS_PROFILE"
	fi
	gsettings set "org.gnome.Ptyxis.Profile:/org/gnome/Ptyxis/Profiles/$PTYXIS_PROFILE/" palette "$PTYXIS_PALETTE"
	fc-list -q 'JetBrains Mono' 2>/dev/null && font='JetBrains Mono'
	gsettings set org.gnome.Ptyxis use-system-font false
	gsettings set org.gnome.Ptyxis font-name "$font 11"
}

# Firefox reads userChrome.css from each profile at startup. The theme goes
# to the profiles the installs use, a foreign userChrome.css is moved aside
# like gtk.css. Profiles appear on the first start, so a fresh system needs
# one Firefox start and another run.
FIREFOX_ROOTS=(
	"${XDG_CONFIG_HOME:-$HOME/.config}/mozilla/firefox"
	"$HOME/.mozilla/firefox"
	"$HOME/.var/app/org.mozilla.firefox/config/mozilla/firefox"
	"$HOME/.var/app/org.mozilla.firefox/.mozilla/firefox"
)
FIREFOX_PREF='user_pref("toolkit.legacyUserProfileCustomizations.stylesheets", true); // kait2en-vornehm'

# Lists the profile directories one per line. A missing profiles.ini only
# means that Firefox never ran from that location.
firefox_profiles() {
	local root path
	for root in "${FIREFOX_ROOTS[@]}"; do
		[[ -e "$root/profiles.ini" ]] || continue
		if [[ ! -r "$root/profiles.ini" ]]; then
			record_error "cannot read $root/profiles.ini, its Firefox profiles stay unthemed"
			continue
		fi
		{
			if [[ -r "$root/installs.ini" ]]; then
				sed -n 's/^Default=//p' "$root/installs.ini"
			fi
			awk -F= '/^\[/ { path = "" } $1 == "Path" { path = $2 }
				$1 == "Default" && $2 == 1 && path != "" { print path }' "$root/profiles.ini"
		} | sort -u | while read -r path; do
			[[ $path == /* ]] || path="$root/$path"
			if [[ -d $path ]]; then printf '%s\n' "$path"; fi
		done
	done
}

# Only a userChrome.css that consists of the import line is ours.
is_own_userchrome() {
	[[ -f $1 && $(<"$1") == "$IMPORT_LINE" ]]
}

install_firefox_profile() {
	local profile=$1 chrome=$1/chrome theme
	install -d -m 0755 "$chrome"
	install -m 0644 "$THEME_SRC/firefox/kait2en-vornehm.css" "$chrome/kait2en-vornehm.css"
	if [[ -e "$chrome/userChrome.css" ]] && ! is_own_userchrome "$chrome/userChrome.css"; then
		if [[ ! -e "$chrome/userChrome.css.kait2en-vornehm-backup" ]]; then
			mv "$chrome/userChrome.css" "$chrome/userChrome.css.kait2en-vornehm-backup"
		fi
		info "previous userChrome.css saved in $chrome"
	fi
	printf '%s\n' "$IMPORT_LINE" >"$chrome/userChrome.css"
	if [[ ! -e "$profile/user.js" ]] || ! grep -qxF "$FIREFOX_PREF" "$profile/user.js"; then
		printf '%s\n' "$FIREFOX_PREF" >>"$profile/user.js"
	fi
	# prefs.js appears on the first start of the profile
	if [[ -e "$profile/prefs.js" ]]; then
		theme=$(sed -n 's/^user_pref("extensions.activeThemeID", "\(.*\)");$/\1/p' "$profile/prefs.js")
		[[ -z $theme || $theme == default-theme@mozilla.org ]] ||
			warn "Firefox profile ${profile##*/} uses another theme, pick \"System theme — auto\" for KaiT2en Vornehm"
	fi
}

remove_firefox_profile() {
	local profile=$1 chrome=$1/chrome
	rm -f "$chrome/kait2en-vornehm.css"
	if [[ -e "$chrome/userChrome.css.kait2en-vornehm-backup" ]]; then
		mv -f "$chrome/userChrome.css.kait2en-vornehm-backup" "$chrome/userChrome.css"
	elif is_own_userchrome "$chrome/userChrome.css"; then
		rm -f "$chrome/userChrome.css"
	fi
	if [[ -d $chrome ]]; then
		find "$chrome" -maxdepth 0 -empty -delete
	fi
	if [[ -e "$profile/user.js" ]]; then
		sed -i "\\|// kait2en-vornehm\$|d" "$profile/user.js"
		[[ -s "$profile/user.js" ]] || rm -f "$profile/user.js"
	fi
}

# Each profile is its own step, a broken one is logged and the others and
# the rest of the theme still go through.
for_each_firefox_profile() {
	local action=$1 label=$2 list profile
	list=$(firefox_profiles)
	if [[ -z $list ]]; then
		[[ $action == remove_firefox_profile ]] ||
			warn "no Firefox profile yet, start Firefox once and run this script again"
		return 0
	fi
	while read -r profile; do
		run_step "$label ${profile##*/}" "$action" "$profile"
	done <<<"$list"
}

restore_look_settings() {
	local schema key value
	[[ -r "$SETTINGS_PREVIOUS" ]] || return 0
	while read -r schema key value; do
		gsettings set "$schema" "$key" "$value"
	done <"$SETTINGS_PREVIOUS"
}

# libadwaita only reads the gtk.css in the user config directory. A foreign
# gtk.css is moved aside and comes back on --remove.
if [[ $remove == true ]]; then
	info "removing KaiT2en Vornehm"
	if [[ -r "$ICON_PREVIOUS" ]]; then
		gsettings set org.gnome.desktop.interface icon-theme "$(<"$ICON_PREVIOUS")"
	fi
	rm -rf "$ICON_DIR"
	restore_look_settings
	rm -f "$PTYXIS_PALETTES/$PTYXIS_PALETTE.palette"
	for_each_firefox_profile remove_firefox_profile "removing KaiT2en Vornehm from Firefox profile"
	rm -rf "$BG_DIR"
	rm -f "$BG_PROPS"
	set_extension_enabled false
	rm -rf "$EXT_DIR"
	rm -f "$GTK4_DIR/kait2en-vornehm.css"
	if [[ -f "$GTK4_BACKUP" ]]; then
		mv -f "$GTK4_BACKUP" "$GTK4_CSS"
	elif [[ -f "$GTK4_CSS" ]] && grep -qxF "$IMPORT_LINE" "$GTK4_CSS"; then
		rm -f "$GTK4_CSS"
	fi
	info "restart running apps and log out once to drop KaiT2en Vornehm"
	exit 0
fi

info "installing KaiT2en Vornehm"
install -d -m 0755 "$GTK4_DIR"
install -m 0644 "$THEME_SRC/gtk-4.0/kait2en-vornehm.css" "$GTK4_DIR/kait2en-vornehm.css"

if [[ -f "$GTK4_CSS" ]] && ! grep -qxF "$IMPORT_LINE" "$GTK4_CSS"; then
	[[ -e "$GTK4_BACKUP" ]] || mv "$GTK4_CSS" "$GTK4_BACKUP"
	info "previous gtk.css saved as $GTK4_BACKUP"
fi
printf '%s\n' "$IMPORT_LINE" >"$GTK4_CSS"

rm -rf "$EXT_DIR"
install -d -m 0755 "$EXT_DIR"
install -m 0644 "$THEME_SRC/gnome-shell/$EXT_UUID"/* "$EXT_DIR/"
set_extension_enabled true
[[ $(gsettings get org.gnome.shell disable-user-extensions) == false ]] ||
	warn "user extensions are switched off in GNOME, the shell part stays inactive"

build_icon_theme
current_icons=$(gsettings get org.gnome.desktop.interface icon-theme)
current_icons=${current_icons//\'/}
[[ $current_icons == kait2en-vornehm ]] || printf '%s\n' "$current_icons" >"$ICON_PREVIOUS"
gsettings set org.gnome.desktop.interface icon-theme kait2en-vornehm

install_backgrounds
prepare_ptyxis
save_look_settings
apply_ptyxis
for_each_firefox_profile install_firefox_profile "installing KaiT2en Vornehm in Firefox profile"
gsettings set org.gnome.desktop.background picture-uri "file://$BG_DIR/$BG_DEFAULT_LIGHT"
gsettings set org.gnome.desktop.background picture-uri-dark "file://$BG_DIR/$BG_DEFAULT"
gsettings set org.gnome.desktop.background picture-options zoom
gsettings set org.gnome.desktop.interface color-scheme prefer-dark

# Flatpak apps see the host config directory only with this override.
if command -v flatpak >/dev/null 2>&1; then
	flatpak override --user --filesystem=xdg-config/gtk-4.0:ro
fi

info "restart running apps and Firefox to see KaiT2en Vornehm, log out once for the shell part"
