#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
set -euo pipefail

APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"
require_root
install_kait2en_fonts
require_fedora
require_command dnf make sudo

dnf install -y cargo gcc gtk4-devel libadwaita-devel

target_user=${SUDO_USER:-}
[[ -n "$target_user" && "$target_user" != root ]] ||
	fail "t2-power-explorer must be built for the user who invoked sudo"
build_cargo_project "$APP_DIR" "$target_user"
make -C "$APP_DIR" install
info "t2-power-explorer installed"
