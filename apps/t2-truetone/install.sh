#!/usr/bin/env bash
# Installs T2 TrueTone. A first installation leaves it off, the user turns it
# on in the app. Updates keep the user's configuration and service state.
# It replaces the former t2-autobrightness service and keeps its users on
# automatic brightness.
set -uo pipefail
APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"
require_root
require_fedora
has_ambient_light_sensor || fail "this Mac has no ambient light sensor"

UNIT=t2-truetone.service
OLD_UNIT=kait2en-t2-autobrightness.service
OLD_BIN=/usr/local/bin/t2-autobrightness
OLD_UNIT_FILE=/usr/local/lib/systemd/user/$OLD_UNIT
TARGET_USER=${SUDO_USER:-}

# Runs a command as the invoking user, in the running session when there is
# one. Without a session nothing runs now, the next login picks it up.
in_session() {
	local uid runtime
	[[ -n "$TARGET_USER" && "$TARGET_USER" != root ]] || return 0
	uid=$(id -u "$TARGET_USER")
	runtime=/run/user/$uid
	[[ -S "$runtime/bus" ]] || return 0
	runuser -u "$TARGET_USER" -- env XDG_RUNTIME_DIR="$runtime" DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
		bash -c "$1"
}

user_home() {
	getent passwd "$TARGET_USER" | cut -d: -f6
}

# Enabled for this user only, so the app can turn it off again.
enable_for_user() {
	local home wants
	home=$(user_home)
	[[ -n "$home" ]] || return 1
	wants="$home/.config/systemd/user/graphical-session.target.wants"
	runuser -u "$TARGET_USER" -- mkdir -p "$wants"
	runuser -u "$TARGET_USER" -- ln -sf "/usr/local/lib/systemd/user/$UNIT" "$wants/$UNIT"
}

# A configuration is only written when the user has none, never on update.
write_config_if_missing() {
	local home dir
	home=$(user_home)
	[[ -n "$home" ]] || return 1
	dir="$home/.config/t2-truetone"
	[[ -e "$dir/config.toml" ]] && return 0
	runuser -u "$TARGET_USER" -- mkdir -p "$dir"
	runuser -u "$TARGET_USER" -- tee "$dir/config.toml" >/dev/null <<-'EOF'
		true_tone = false
		night = false
		auto_brightness = true
	EOF
}

# Stopping the old service gives GNOME its brightness settings back. The
# learned curve stays in the user's state directory, t2-truetone takes it
# over on its first start. The shell extension has the same path in both
# and was already replaced by make install.
replace_autobrightness() {
	local was_enabled=0
	[[ -e "$OLD_BIN" || -e "$OLD_UNIT_FILE" ]] || return 0
	info "replacing t2-autobrightness with t2-truetone"
	if [[ -e "$OLD_UNIT_FILE" ]]; then
		systemctl --global is-enabled --quiet "$OLD_UNIT" && was_enabled=1
		in_session "systemctl --user stop $OLD_UNIT" || warn "could not stop $OLD_UNIT in the running session"
		systemctl --global disable "$OLD_UNIT" || warn "could not disable $OLD_UNIT globally"
	fi
	rm -f "$OLD_BIN" "$OLD_UNIT_FILE" /var/lib/kait2en-installer/applied/install-autobrightness.sh
	in_session "systemctl --user daemon-reload" || warn "could not reload the user units of $TARGET_USER"
	if (( was_enabled )) && [[ -n "$TARGET_USER" && "$TARGET_USER" != root ]]; then
		write_config_if_missing || warn "could not write the t2-truetone configuration for $TARGET_USER"
		if enable_for_user; then
			in_session "systemctl --user daemon-reload && systemctl --user start $UNIT" ||
				warn "could not start $UNIT in the running session, it starts with the next login"
		else
			warn "could not enable $UNIT for $TARGET_USER"
		fi
		info "automatic brightness now runs in t2-truetone for $TARGET_USER"
		info "other users turn it on again in the T2 TrueTone app"
	fi
}

[[ -n "$TARGET_USER" && "$TARGET_USER" != root ]] || fail "t2-truetone must be built for the user who invoked sudo"
build_cargo_project "$APP_DIR" "$TARGET_USER" || fail "t2-truetone build failed"
make -C "$APP_DIR" install || fail "t2-truetone installation failed"
replace_autobrightness
in_session "systemctl --user daemon-reload && systemctl --user try-restart $UNIT" ||
	warn "could not restart $UNIT in the running session, the update takes effect with the next login"
info "t2-truetone installed"
