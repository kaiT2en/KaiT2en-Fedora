#!/usr/bin/env bash
# Failure injection: no root privileges, host installation or hardware needed.
set -Eeuo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
work=$(mktemp -d /tmp/kait2en-error-test.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
export KAIT2EN_INSTALL_ERRORS="$work/errors"
export TEST_LIB="$repo_root/scripts/fedora/lib.sh"

bash -c '
source "$TEST_LIB"
broken() { false; printf "UNSAFE\n"; }
run_step "injected failure" broken
run_step "independent step" printf "CONTINUED\n"
run_step "child failure" bash -c '\''source "$TEST_LIB"; fail "nested failure"'\''
run_step "last step" printf "FINISHED\n"
' >"$work/output" 2>&1
! grep -q UNSAFE "$work/output"
grep -q CONTINUED "$work/output"
grep -q FINISHED "$work/output"
grep -q 'injected failure failed' "$work/errors"
grep -q 'nested failure' "$work/errors"

# A parent's EXIT cleanup must not run at the end of every isolated step.
TEST_CLEANUP="$work/cleanup" bash -c '
source "$TEST_LIB"
trap '\''printf "cleanup\n" >>"$TEST_CLEANUP"'\'' EXIT
run_step one true
run_step two true
'
[[ $(wc -l <"$work/cleanup") -eq 1 ]]

if bash -c 'source "$TEST_LIB"; INSTALL_REPORT_OWNER=1; warn "reported warning"; printf "FINISHED\n"' >"$work/summary" 2>&1; then
	printf 'error: installation with a recorded hard failure returned success\n' >&2
	exit 1
fi
grep -q FINISHED "$work/summary"
grep -q 'installation completed with errors' "$work/summary"
grep -q 'nested failure' "$work/summary"
grep -q 'reported warning' "$work/summary"

# A lone warning (e.g. a step skipping itself on unsupported hardware) must
# not fail the whole installation, or the auto-installer never marks it
# complete and re-runs it forever.
if ! KAIT2EN_INSTALL_ERRORS="$work/warn-only-errors" bash -c 'source "$TEST_LIB"; INSTALL_REPORT_OWNER=1; warn "just a warning"; printf "FINISHED\n"' >"$work/warn-only" 2>&1; then
	printf 'error: a lone warning failed the installation\n' >&2
	exit 1
fi
grep -q FINISHED "$work/warn-only"
grep -q 'completed with warnings' "$work/warn-only"
grep -q 'just a warning' "$work/warn-only"

KAIT2EN_INSTALL_ERRORS="$work/clean-errors" bash -c 'source "$TEST_LIB"; INSTALL_REPORT_OWNER=1; run_step success true' >"$work/clean" 2>&1
grep -q 'without recorded errors' "$work/clean"
printf 'Installer failure collection and continuation: PASS\n'
