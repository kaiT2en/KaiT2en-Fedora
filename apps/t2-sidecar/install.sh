#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
set -euo pipefail
APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"
require_root
require_fedora
require_command dnf install
dnf install -y python3-gobject
install -d -m 0755 /usr/local/bin /usr/local/libexec /usr/local/lib/systemd/user
install -m 0755 "$APP_DIR/t2-sidecar" /usr/local/bin/t2-sidecar
install -m 0755 "$APP_DIR/t2-sidecar-display" /usr/local/libexec/t2-sidecar-display
install -m 0644 "$APP_DIR/t2-sidecar.service" /usr/local/lib/systemd/user/t2-sidecar.service
install -m 0644 "$APP_DIR/t2-sidecar-sunshine.service" /usr/local/lib/systemd/user/t2-sidecar-sunshine.service
info "t2-sidecar installed"
