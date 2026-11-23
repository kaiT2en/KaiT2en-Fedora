#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
test_dir=$(mktemp -d /tmp/kait2en-run-step-test.XXXXXX)

# Model main installer -> install-apps -> standalone component installer with
# the same shared ledger used in production.
export KAIT2EN_INSTALL_ERRORS="$test_dir/errors"
source "$repo_root/scripts/fedora/lib.sh"
trap 'rm -rf -- "$test_dir"' EXIT

failing_component() {
	run_step "simulated inner build failure" false
	local component_status=$STEP_STATUS
	if (( component_status != 0 )); then
		return "$component_status"
	fi
}

downstream_step() {
	printf 'continued\n' >"$test_dir/downstream-ran"
}

{
	run_step "simulated touchbar installer" failing_component
	component_status=$STEP_STATUS
	(( component_status != 0 ))

	run_step "simulated later KAIT2EN step" downstream_step
	(( STEP_STATUS == 0 ))
} >"$test_dir/run.log" 2>&1
[[ -f "$test_dir/downstream-ran" ]]
[[ -s "$KAIT2EN_INSTALL_HARD_ERRORS" ]]

echo "Nested installer failure-continuation checks passed."
