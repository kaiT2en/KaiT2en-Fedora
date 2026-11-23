#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command install systemctl readlink timeout

SCRIPT_SRC="$REPO_ROOT/scripts/fedora/kait2en-suspend.sh"

[[ -f "$SCRIPT_SRC" ]] || fail "missing $SCRIPT_SRC"

info "installing Kait2en suspend helper"
install -d -o root -g root -m 0755 /usr/local/libexec/kait2en
install -o root -g root -m 0755 "$SCRIPT_SRC" /usr/local/libexec/kait2en/kait2en-suspend.sh

tee /etc/systemd/system/kait2en-suspend.service >/dev/null <<'EOF'
[Unit]
Description=Kait2en T2 suspend and resume hardware fixes
Before=sleep.target
After=NetworkManager.service t2-services-suspend.service
StopWhenUnneeded=yes

[Service]
Type=oneshot
RemainAfterExit=yes
TimeoutStopSec=150
ExecStart=/usr/local/libexec/kait2en/kait2en-suspend.sh pre
ExecStop=/usr/local/libexec/kait2en/kait2en-suspend.sh post

[Install]
WantedBy=sleep.target
EOF
chmod 0644 /etc/systemd/system/kait2en-suspend.service

systemctl daemon-reload
systemctl enable kait2en-suspend.service

info "Kait2en suspend helper installed"
