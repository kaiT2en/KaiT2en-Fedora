#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install systemctl authselect make sudo checkmodule semodule_package semodule

BIN_USER="${SUDO_USER:-}"
[[ -n "$BIN_USER" && "$BIN_USER" != root ]] ||
	fail "t2-touchid must be built by the user who invoked sudo"

SERVICE_SRC="$REPO_ROOT/systemd/kait2en-t2-touchid.service"
DROPIN_SRC="$REPO_ROOT/systemd/fprintd-kait2en-t2-touchid.conf"
CONFIG_SRC="$REPO_ROOT/data/kait2en/t2-touchid.conf"
CONFIG_DST="/etc/kait2en/t2-touchid.conf"

[[ -f "$SERVICE_SRC" ]] || fail "missing $SERVICE_SRC"
[[ -f "$DROPIN_SRC" ]] || fail "missing $DROPIN_SRC"
[[ -f "$CONFIG_SRC" ]] || fail "missing $CONFIG_SRC"

info "installing the Touch ID bridge"
if ! sudo -H -u "$BIN_USER" make -C "$REPO_ROOT/apps/t2-touchid" build; then
	fail "t2-touchid build failed"
fi
if ! make -C "$REPO_ROOT/apps/t2-touchid" install; then
	fail "t2-touchid installation failed"
fi

install -d -o root -g root -m 0755 /etc/kait2en
if [[ -e "$CONFIG_DST" ]]; then
	info "keeping the existing $CONFIG_DST"
else
	install -o root -g root -m 0644 "$CONFIG_SRC" "$CONFIG_DST"
fi
# The bridge binds the macOS-enrolled fingers to this account by itself, so
# nobody has to sit at the sensor during an installation or an update.
if grep -q '^T2_TOUCHID_BIND_USER=' "$CONFIG_DST"; then
	sed -i "s/^T2_TOUCHID_BIND_USER=.*/T2_TOUCHID_BIND_USER=$BIN_USER/" "$CONFIG_DST"
else
	printf 'T2_TOUCHID_BIND_USER=%s\n' "$BIN_USER" >>"$CONFIG_DST"
fi

info "installing the SELinux policy for the fprintd socket"
if ! make -C "$REPO_ROOT/selinux"; then
	warn "could not build the SELinux module; fprintd will be denied the socket"
elif ! semodule -i "$REPO_ROOT/selinux/kait2en-t2-touchid.pp"; then
	warn "could not install the SELinux module; fprintd will be denied the socket"
fi

install -o root -g root -m 0644 "$SERVICE_SRC" /etc/systemd/system/kait2en-t2-touchid.service
install -d -o root -g root -m 0755 /etc/systemd/system/fprintd.service.d
install -o root -g root -m 0644 "$DROPIN_SRC" \
	/etc/systemd/system/fprintd.service.d/kait2en-t2-touchid.conf

systemctl daemon-reload

# A directory left over from an earlier install carries the old label. Drop it
# and let systemd recreate it, which applies the type from the policy module.
if ! systemctl stop kait2en-t2-touchid.service; then
	info "the bridge was not running"
fi
rm -rf /run/t2-touchid

if ! systemctl enable --now kait2en-t2-touchid.service; then
	warn "could not start kait2en-t2-touchid.service; start it by hand"
fi

# An fprintd that was already running knows nothing of the drop-in yet.
if ! systemctl try-restart fprintd.service; then
	warn "could not restart fprintd; it picks the reader up after a reboot"
fi

if ! authselect enable-feature with-fingerprint; then
	warn "could not enable the fingerprint PAM feature; login and sudo will keep asking for the password only"
fi

info "Touch ID bridge installed; the fingers enrolled under macOS are bound to $BIN_USER as soon as the sensor is reachable"
