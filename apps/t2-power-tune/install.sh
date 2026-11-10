#!/usr/bin/env bash
set -euo pipefail
APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"
require_root
install_kait2en_fonts
require_fedora
require_command dnf install systemctl
dnf install -y gtk4 libadwaita pciutils polkit python3-gobject
install -d -m 0755 /usr/local/bin /usr/local/libexec /usr/local/share/applications \
    /usr/local/share/icons/hicolor/scalable/apps /usr/share/polkit-1/actions
install -m 0755 "$APP_DIR/t2-power-tune.py" /usr/local/bin/t2-power-tune
install -m 0755 "$APP_DIR/t2-power-tune-helper" /usr/local/libexec/t2-power-tune-helper
install -m 0755 "$APP_DIR/t2-power-tune-status" /usr/local/libexec/t2-power-tune-status
rm -f /usr/local/libexec/t2-power-tune-cstates
# An existing selection gets the units of this version, including the
# one that reapplies it after resume.
if [[ -f /etc/systemd/system/kait2en-power-tune.service ]]; then
    /usr/local/libexec/t2-power-tune-helper refresh ||
        warn "could not refresh kait2en-power-tune.service, save the selection again in T2 Power Tune"
fi
install -m 0644 "$APP_DIR/org.t2powertune.policy" /usr/share/polkit-1/actions/
install -m 0644 "$APP_DIR/org.t2powertune.gtk.desktop" /usr/local/share/applications/
install -m 0644 "$APP_DIR/org.t2powertune.gtk.svg" /usr/local/share/icons/hicolor/scalable/apps/
gtk-update-icon-cache --force --ignore-theme-index /usr/local/share/icons/hicolor
update-desktop-database /usr/local/share/applications
info "t2-power-tune installed"
