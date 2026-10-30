#!/usr/bin/env bash
set -Eeuo pipefail

APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"

require_root
require_repo_root
require_fedora
require_command dnf sudo systemctl udevadm getent groupadd usermod

target_user="${SUDO_USER:-}"
[[ -n "$target_user" && "$target_user" != root ]] ||
	fail "run this installer through sudo as the desktop user"
target_uid=$(id -u "$target_user")

user_systemctl() {
	local runtime="/run/user/$target_uid"
	[[ -S "$runtime/bus" ]] || return 1
	sudo -H -u "$target_user" env \
		XDG_RUNTIME_DIR="$runtime" \
		DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
		systemctl --user "$@"
}

device_group_is_live() {
	local group_id manager_pid

	group_id=$(getent group kait2en-touchbar | cut -d: -f3) || return 1
	[[ "$group_id" =~ ^[0-9]+$ ]] || return 1
	manager_pid=$(systemctl show "user@$target_uid.service" --property=MainPID --value 2>/dev/null) || return 1
	[[ "$manager_pid" =~ ^[1-9][0-9]*$ && -r "/proc/$manager_pid/status" ]] || return 1
	awk -v wanted="$group_id" '
		$1 == "Groups:" {
			for (field = 2; field <= NF; field++)
				if ($field == wanted)
					found = 1
		}
		END { exit !found }
	' "/proc/$manager_pid/status"
}

user_manager_is_running() {
	systemctl is-active --quiet "user@$target_uid.service"
}

# The daemon can open the panel once its user manager carries the device
# group: either a manager that already has it, or one that starts later.
device_access_ready() {
	! user_manager_is_running || device_group_is_live
}

install_dependencies() {
	dnf install -y adwaita-sans-fonts cargo gcc libinput-devel make pkgconf-pkg-config rust systemd-devel
	require_command cargo make
}

prepare_device_access() {
	getent group kait2en-touchbar >/dev/null || groupadd --system kait2en-touchbar
	usermod -aG kait2en-touchbar "$target_user"
}

# activity_backlight became activity_escape when the keyboard backlight moved
# to t2-kbd-backlight. The daemon refuses unknown keys, so rename it in place.
migrate_config() {
	local file home
	home=$(getent passwd "$target_user" | cut -d: -f6)
	for file in /etc/kait2en/touchbar.toml "$home/.config/kait2en-touchbar/config.toml"; do
		if [[ ! -f "$file" ]] || ! grep -Eq '^[[:space:]]*activity_backlight[[:space:]]*=' "$file"; then
			continue
		fi
		info "renaming activity_backlight to activity_escape in $file"
		sed -Ei 's/^([[:space:]]*)activity_backlight([[:space:]]*=)/\1activity_escape\2/' "$file"
	done
}

build_and_install() {
	build_cargo_project "$APP_DIR" "$target_user"
	make -C "$APP_DIR" install
}

configure_system() {
	if systemctl list-unit-files tiny-dfr.service --no-legend 2>/dev/null | grep -q tiny-dfr; then
		systemctl disable --now tiny-dfr.service || warn "could not stop system tiny-dfr.service"
	fi
	if user_systemctl is-enabled tiny-dfr.service &>/dev/null ||
		user_systemctl is-active tiny-dfr.service &>/dev/null; then
		user_systemctl disable --now tiny-dfr.service || warn "could not stop user tiny-dfr.service"
	fi
	if user_systemctl is-enabled react-drm.service &>/dev/null ||
		user_systemctl is-active react-drm.service &>/dev/null; then
		user_systemctl disable --now react-drm.service || warn "could not stop react-drm.service"
	fi

	systemctl daemon-reload
	# The attach service only switches the panel while the daemon is enabled,
	# so enable the daemon first.
	systemctl --global enable kait2en-touchbar.service
	systemctl enable kait2en-touchbar-attach.service
	udevadm control --reload
	udevadm trigger --subsystem-match=usb --attr-match=idVendor=05ac --attr-match=idProduct=8102 --action=add
	if device_access_ready; then
		udevadm trigger --subsystem-match=usb --attr-match=idVendor=05ac --attr-match=idProduct=8302 --action=add
		# Freshly built modules may only load after a reboot. The attach
		# helper then falls back to the firmware row by itself.
		systemctl restart kait2en-touchbar-attach.service ||
			warn "the Touch Bar display is not ready yet. It switches over after the next reboot"
	else
		# First install: the running session cannot open the panel before the
		# reboot, so keep Apple's firmware row instead of switching the panel
		# away and back. The enabled attach service switches it at boot.
		info "keeping the firmware Touch Bar until the next reboot"
	fi
	udevadm settle
	udevadm trigger --subsystem-match=drm --action=add
	udevadm trigger --subsystem-match=input --action=add
	udevadm trigger --subsystem-match=backlight --action=add
	udevadm trigger --subsystem-match=hid --action=add
	udevadm trigger --subsystem-match=misc --action=add
}

activate_user_service() {
	if ! user_manager_is_running; then
		info "kait2en-touchbar will start at the next graphical login"
	elif ! device_group_is_live; then
		# A logout does not necessarily restart user@UID.service. Never leave the
		# panel in DRM configuration 2 while that manager still lacks access.
		# Like react-drm did: only enable it. configure_system left the panel
		# on the firmware row, so there is nothing to switch back here.
		if [[ -S "/run/user/$target_uid/bus" ]]; then
			user_systemctl stop kait2en-touchbar.service || true
		fi
		warn "reboot once so kait2en-touchbar receives its device-access group. The firmware Touch Bar remains available until then"
	elif [[ -S "/run/user/$target_uid/bus" ]] &&
		user_systemctl is-active --quiet graphical-session.target; then
		user_systemctl daemon-reload
		user_systemctl restart kait2en-touchbar.service
	else
		info "kait2en-touchbar will start at the next graphical login"
	fi
}

run_step "install kait2en-touchbar dependencies" install_dependencies
if (( STEP_STATUS == 0 )); then
	run_step "prepare kait2en-touchbar device access" prepare_device_access
fi
if (( STEP_STATUS == 0 )); then
	run_step "build and install kait2en-touchbar" build_and_install
fi
if (( STEP_STATUS == 0 )); then
	run_step "migrate kait2en-touchbar configuration" migrate_config
fi
if (( STEP_STATUS == 0 )); then
	run_step "configure kait2en-touchbar integration" configure_system
fi
if (( STEP_STATUS == 0 )); then
	run_step "activate kait2en-touchbar user service" activate_user_service
fi
if (( STEP_STATUS == 0 )); then
	info "kait2en-touchbar installed"
else
	exit "$STEP_STATUS"
fi
