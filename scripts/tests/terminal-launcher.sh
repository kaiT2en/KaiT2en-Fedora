#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
launcher="$repo_root/auto-installer/runtime/kait2en-launch-terminal"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-terminal-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
log="$work/terminal.log"

run_case() {
	local terminal=$1 expected=$2
	local fake_bin="$work/$terminal"
	mkdir -p "$fake_bin"
	cat >"$fake_bin/$terminal" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$KAIT2EN_TEST_LOG"
EOF
	ln -s /bin/bash "$fake_bin/bash"
	chmod 0755 "$fake_bin/$terminal"
	PATH="$fake_bin" \
		KAIT2EN_INSTALL_COMMAND=/test/kait2en-install \
		KAIT2EN_TEST_LOG="$log" \
		bash "$launcher"
	grep -Fxq -- "$expected" "$log"
}

run_case ptyxis '--standalone -- /test/kait2en-install'
run_case konsole '-e /test/kait2en-install'
run_case cosmic-term '-e /test/kait2en-install'
