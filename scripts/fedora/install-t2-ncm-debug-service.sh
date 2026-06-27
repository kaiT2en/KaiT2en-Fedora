#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install systemctl

SERVICE_SRC="$REPO_ROOT/systemd/kait2en-t2-ncm-down.service"
SCRIPT_SRC="$REPO_ROOT/scripts/fedora/kait2en-t2-ncm-down.sh"

[[ -f "$SERVICE_SRC" ]] || fail "missing $SERVICE_SRC"
[[ -f "$SCRIPT_SRC" ]] || fail "missing $SCRIPT_SRC"

info "installing Kait2en T2 CDC-NCM debug interface helper"
install -d -o root -g root -m 0755 /usr/local/libexec/kait2en
install -o root -g root -m 0755 "$SCRIPT_SRC" /usr/local/libexec/kait2en/kait2en-t2-ncm-down.sh
install -o root -g root -m 0644 "$SERVICE_SRC" /etc/systemd/system/kait2en-t2-ncm-down.service

systemctl disable kait2en-t2-ncm-down.service >/dev/null 2>&1 || true
systemctl daemon-reload

info "Kait2en T2 CDC-NCM debug interface helper installed"
