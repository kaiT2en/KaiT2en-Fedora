#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install systemctl

SERVICE_SRC="$REPO_ROOT/systemd/kait2en-suspend.service"
SCRIPT_SRC="$REPO_ROOT/scripts/fedora/kait2en-suspend.sh"

[[ -f "$SERVICE_SRC" ]] || fail "missing $SERVICE_SRC"
[[ -f "$SCRIPT_SRC" ]] || fail "missing $SCRIPT_SRC"

info "installing Kait2en suspend helper"
install -d -o root -g root -m 0755 /usr/local/libexec/kait2en
install -o root -g root -m 0755 "$SCRIPT_SRC" /usr/local/libexec/kait2en/kait2en-suspend.sh
install -o root -g root -m 0644 "$SERVICE_SRC" /etc/systemd/system/kait2en-suspend.service

systemctl daemon-reload
systemctl enable kait2en-suspend.service

info "Kait2en suspend helper installed"
