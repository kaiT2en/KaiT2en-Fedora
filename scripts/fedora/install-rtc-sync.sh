#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command systemctl hwclock chronyc

# The kernel's NTP RTC sync never writes t2smc on x86 (it stops at the legacy
# CMOS clock), so write it once chrony has synchronized at boot. t2smc itself
# synchronizes the RTC before suspend and at shutdown.
info "installing Kait2en SMC RTC sync"
tee /etc/systemd/system/kait2en-rtc-sync.service >/dev/null <<'EOF'
[Unit]
Description=Kait2en write system time to the T2 SMC RTC
After=chronyd.service

[Service]
Type=exec
RemainAfterExit=yes
ExecStart=/bin/sh -c 'chronyc waitsync 180 && hwclock --systohc --utc'

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 /etc/systemd/system/kait2en-rtc-sync.service

systemctl daemon-reload
systemctl enable kait2en-rtc-sync.service

info "Kait2en SMC RTC sync installed"
