#!/usr/bin/env bash
#
# Removes the system integration installed by react-drm.
# Project files, dependencies and user group memberships are left unchanged.
#
# Author: André Eikmeyer (dev@deqrocks)
# Date: 2026-06-14
#
# This script is provided without warranty. Use it at your own risk.
# The author and project contributors are not responsible for data loss,
# hardware damage, system failure, or any other consequences of its use.

set -Eeuo pipefail
shopt -s nullglob

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
SERVICE_FILE="$HOME/.config/systemd/user/react-drm.service"
UDEV_RULE="/etc/udev/rules.d/99-react-drm.rules"
LEGACY_UDEV_RULE="/etc/udev/rules.d/99-react-drm-uinput.rules"

info(){ printf '[uninstall] %s\n' "$*"; }
fail(){ printf '[uninstall] error: %s\n' "$1" >&2; exit 1; }
trap 'printf "[uninstall] fatal: line %s: %s\n" "$LINENO" "$BASH_COMMAND" >&2; exit 1' ERR

confirm_uninstall() {
  local answer cmd
  [[ $EUID -ne 0 ]] || fail "run this script as your regular user, not as root"
  for cmd in sudo systemctl udevadm; do
    command -v "$cmd" >/dev/null 2>&1 || fail "required command is missing: $cmd"
  done
  systemctl --user show-environment >/dev/null ||
    fail "unable to connect to the systemd user manager"
  cat <<'EOF'
This removes the react-drm user service and udev rules and restores the
firmware Touch Bar interface.

Project files, npm dependencies, system packages and video/input group
memberships are not removed.
EOF
  printf '\nType UNINSTALL to continue, or anything else to cancel: '
  IFS= read -r answer || fail "uninstallation cancelled"
  [[ "$answer" == UNINSTALL ]] || fail "uninstallation cancelled"
  sudo -v || fail "unable to acquire administrative privileges"
}

control_center_running() {
  local proc cwd cmdline
  for proc in /proc/[0-9]*; do
    cwd=$(readlink "$proc/cwd" 2>/dev/null) || continue
    [[ "${cwd##*/}" == linux-touchbar-control-center ]] || continue
    cmdline=$(tr '\0' ' ' <"$proc/cmdline" 2>/dev/null) || continue
    [[ "$cmdline" == *index.tsx* ]] && return 0
  done
  return 1
}

remove_service() {
  if [[ -e "$SERVICE_FILE" ]] ||
    systemctl --user is-active --quiet react-drm.service ||
    systemctl --user is-enabled --quiet react-drm.service; then
    info "Stopping and disabling react-drm.service"
    systemctl --user disable --now react-drm.service
  fi
  systemctl --user is-active --quiet react-drm.service &&
    fail "react-drm.service did not stop"
  control_center_running &&
    fail "a manually started react-drm control center is still running"

  info "Restoring the firmware Touch Bar interface"
  [[ -x "$SCRIPT_DIR/system/react-drm-tb-detach" ]] ||
    fail "system/react-drm-tb-detach is missing or not executable"
  "$SCRIPT_DIR/system/react-drm-tb-detach" ||
    fail "unable to restore the firmware Touch Bar interface"

  rm -f "$SERVICE_FILE"
  systemctl --user daemon-reload
}

remove_udev_rules() {
  info "Removing react-drm udev rules"
  sudo rm -f "$UDEV_RULE" "$LEGACY_UDEV_RULE"
  sudo udevadm control --reload
  sudo udevadm trigger --action=add --subsystem-match=usb --subsystem-match=backlight
  sudo udevadm trigger --action=add --subsystem-match=misc --sysname-match=uinput
}

# Remove the KWin script and disable it in kwinrc so it doesn't
# re-appear after the next login.
remove_kwin_script() {
  command -v kpackagetool6 >/dev/null 2>&1 || return 0
  if kpackagetool6 --type=KWin/Script --list 2>&1 |
     grep -q touchbar_dynamicshortcuts; then
    info "disabling TouchBar Dynamic Shortcuts KWin script"
    kwriteconfig6 --file kwinrc --group Plugins \
      --key touchbar_dynamicshortcutsEnabled false
    info "removing TouchBar Dynamic Shortcuts KWin script"
    kpackagetool6 --type=KWin/Script -r touchbar_dynamicshortcuts ||
      fail "failed to remove KWin script"
  fi
}

main() {
  confirm_uninstall
  remove_service
  remove_udev_rules
  remove_kwin_script
  info "Uninstallation completed successfully"
}

main "$@"
