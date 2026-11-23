#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# Compatibility entry point: this installs only the AVE component.
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"
require_root
require_repo_root
require_fedora

install_ave() {
	local target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] || fail "t2-ave must be built by the invoking user"
	build_cargo_project "$REPO_ROOT/t2-services/t2-ave" "$target_user"
	make -C "$REPO_ROOT/t2-services/t2-ave" install SYSTEMD_UNIT_DIR=/etc/systemd/system
	python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-ave record-source
	systemctl daemon-reload
	systemctl enable kait2en-t2-remote.service
}
run_step "build and install t2-ave" install_ave
