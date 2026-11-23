#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"
require_root
require_repo_root
require_fedora

COMPONENT="$REPO_ROOT/t2-services/t2-touchid"
CONFIG_DST=/etc/kait2en/t2-touchid.conf

build_touchid() {
	local target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] || fail "t2-touchid must be built by the invoking user"
	build_cargo_project "$COMPONENT" "$target_user"
}

install_policy() {
	make -C "$COMPONENT/integration/selinux"
	install -D -m 0644 "$COMPONENT/integration/selinux/kait2en-t2-touchid.pp" /usr/share/selinux/packages/kait2en-t2-touchid.pp
	python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-touchid install-policy
}

install_touchid() {
	local new_config=0
	[[ -e "$CONFIG_DST" ]] || new_config=1
	make -C "$COMPONENT" install SYSTEMD_UNIT_DIR=/etc/systemd/system DATADIR=/usr/share
	# Retain the existing source-installer behaviour only on first install.
	# Updates must never reassign a user's fingerprint binding.
	if (( new_config )) && [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != root ]]; then
		sed -i "s/^T2_TOUCHID_BIND_USER=.*/T2_TOUCHID_BIND_USER=$SUDO_USER/" "$CONFIG_DST"
	fi
	python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-touchid record-source
}

activate_touchid() {
	systemctl daemon-reload
	# Relabel existing runtime files without deleting a live authentication socket.
	if [[ -d /run/t2-touchid ]]; then
		restorecon -R /run/t2-touchid
	fi
	systemctl enable kait2en-t2-touchid.service
	systemctl restart kait2en-t2-touchid.service
}

reload_dbus() {
	if systemctl is-active --quiet dbus-broker.service; then
		systemctl reload dbus-broker.service
	else
		systemctl reload dbus.service
	fi
}

run_step "build t2-touchid" build_touchid
build_status=$STEP_STATUS
run_step "install Touch ID SELinux policy" install_policy
policy_status=$STEP_STATUS
if (( build_status == 0 )); then
	run_step "install t2-touchid files" install_touchid
	if (( STEP_STATUS == 0 )); then
		run_step "reload D-Bus policy" reload_dbus
		if (( policy_status == 0 )); then
			run_step "activate t2-touchid" activate_touchid
			if (( STEP_STATUS == 0 )); then
				run_step "restart active fprintd" systemctl try-restart fprintd.service
					run_step "enable fingerprint PAM feature with ownership receipt" python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-touchid enable-pam
			fi
		else
			record_error "Touch ID activation skipped because its SELinux policy failed"
		fi
	fi
else
	record_error "Touch ID installation skipped because its build failed; existing installation retained"
fi
