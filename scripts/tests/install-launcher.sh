#!/usr/bin/env bash

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
launcher="$repo_root/auto-installer/runtime/kait2en-install"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-install-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

target=7.1.3-200.fc44.x86_64
fake_home="$work/home"
fake_repo="$work/repository"
fake_state="$work/state"
fake_bin="$work/bin"
log="$work/commands.log"
initial_output="$work/initial-output.log"
origin_url_file="$work/origin-url"
canonical_url=https://github.com/kaiT2en/KaiT2en-Fedora.git
git_prefix="-c safe.directory=$fake_repo -C $fake_repo"
mkdir -p "$fake_home" "$fake_repo/.git" "$fake_state" "$fake_bin"
printf '%s\n' "$canonical_url" >"$origin_url_file"
printf 'phase=reboot_pending\ntarget_kernel=%s\n' "$target" >"$fake_state/state"

cat >"$fake_bin/flock" <<'EOF'
#!/usr/bin/env bash
printf 'flock %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
exit 0
EOF
cat >"$fake_bin/uname" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$KAIT2EN_TEST_TARGET"
EOF
cat >"$fake_bin/git" <<'EOF'
#!/usr/bin/env bash
prefix="-c safe.directory=$KAIT2EN_TEST_REPOSITORY -C $KAIT2EN_TEST_REPOSITORY"
case "$*" in
	"$prefix status --porcelain") exit 0 ;;
	"$prefix remote get-url origin") cat "$KAIT2EN_TEST_ORIGIN_URL_FILE" ;;
	"$prefix remote set-url origin "*)
		printf '%s\n' "${@: -1}" >"$KAIT2EN_TEST_ORIGIN_URL_FILE"
		printf 'git %s\n' "$*" >>"$KAIT2EN_TEST_LOG"
		;;
	"ls-remote --exit-code "*) exit 0 ;;
	*) printf 'git %s\n' "$*" >>"$KAIT2EN_TEST_LOG" ;;
esac
EOF
cat >"$fake_bin/sudo" <<'EOF'
#!/usr/bin/env bash
printf 'sudo cwd=%s command=%s\n' "$PWD" "$*" >>"$KAIT2EN_TEST_LOG"
[[ "${KAIT2EN_TEST_INSTALL_FAILS:-0}" == 1 && "$*" == "bash ./scripts/fedora/install.sh" ]] && exit 1
exit 0
EOF
chmod 0755 "$fake_bin"/*

printf 'n\n' |
	env \
		HOME="$fake_home" \
		XDG_RUNTIME_DIR="$work" \
		PATH="$fake_bin:/usr/bin:/bin" \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
		KAIT2EN_TEST_LOG="$log" \
		bash "$launcher" >"$initial_output"

grep -Fq "git $git_prefix pull --ff-only origin main" "$log"
grep -Fq "sudo cwd=$fake_repo command=bash ./scripts/fedora/install.sh" "$log"
grep -Fq 'command=/usr/local/bin/kait2en-prepare --complete' "$log"
grep -Fq "Repository: $fake_repo" "$initial_output"
grep -Fq 'Run kait2en-install at any time to update KaiT2en.' "$initial_output"
grep -Fq 'Reboot once more to start the fully configured system.' "$initial_output"

: >"$log"
printf 'phase=complete\ntarget_kernel=%s\n' "$target" >"$fake_state/state"
printf 'n\n' |
	env \
		HOME="$fake_home" \
		XDG_RUNTIME_DIR="$work" \
		PATH="$fake_bin:/usr/bin:/bin" \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
		KAIT2EN_TEST_LOG="$log" \
		bash "$launcher" >/dev/null
grep -Fq "git $git_prefix pull --ff-only origin main" "$log"
grep -Fq "sudo cwd=$fake_repo command=bash ./scripts/fedora/install.sh" "$log"
! grep -Fq 'kait2en-prepare --complete' "$log"

mkdir -p "$fake_repo/auto-installer/runtime"
cat >"$fake_repo/auto-installer/runtime/kait2en-install" <<'EOF'
#!/usr/bin/env bash
printf 'delegated active=%s\n' \
	"${KAIT2EN_INSTALL_SESSION_ACTIVE:-0}" >>"$KAIT2EN_TEST_LOG"
EOF
: >"$log"
env \
	HOME="$fake_home" \
	XDG_RUNTIME_DIR="$work" \
	PATH="$fake_bin:/usr/bin:/bin" \
	KAIT2EN_STATE_DIR="$fake_state" \
	KAIT2EN_REPOSITORY="$fake_repo" \
	KAIT2EN_TEST_TARGET="$target" \
	KAIT2EN_TEST_REPOSITORY="$fake_repo" \
	KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
	KAIT2EN_TEST_LOG="$log" \
	bash "$launcher" >/dev/null
grep -Fxq 'delegated active=0' "$log"
! grep -Fq 'flock ' "$log"

cp "$launcher" "$fake_repo/auto-installer/runtime/kait2en-install"
: >"$log"
printf 'phase=complete\ntarget_kernel=%s\n' "$target" >"$fake_state/state"
printf 'n\n' |
	env \
		HOME="$fake_home" \
		XDG_RUNTIME_DIR="$work" \
		PATH="$fake_bin:/usr/bin:/bin" \
		KAIT2EN_INSTALL_SESSION_ACTIVE=1 \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
		KAIT2EN_TEST_LOG="$log" \
		bash "$fake_repo/auto-installer/runtime/kait2en-install" >/dev/null
grep -Fq "git $git_prefix pull --ff-only origin main" "$log"
grep -Fq "sudo cwd=$fake_repo command=bash ./scripts/fedora/install.sh" "$log"
! grep -Fq 'flock ' "$log"

: >"$log"
printf '%s\n' 'https://github.com' >"$origin_url_file"
printf 'phase=complete\ntarget_kernel=%s\n' "$target" >"$fake_state/state"
repair_output="$work/repair-output.log"
printf 'n\n' |
	env \
		HOME="$fake_home" \
		XDG_RUNTIME_DIR="$work" \
		PATH="$fake_bin:/usr/bin:/bin" \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
		KAIT2EN_TEST_LOG="$log" \
		bash "$launcher" >"$repair_output"
grep -Fq 'Repairing the KaiT2en origin URL (found: https://github.com).' "$repair_output"
grep -Fq "git $git_prefix remote set-url origin $canonical_url" "$log"
grep -Fxq "$canonical_url" "$origin_url_file"
grep -Fq "git $git_prefix pull --ff-only origin main" "$log"
grep -Fq "sudo cwd=$fake_repo command=bash ./scripts/fedora/install.sh" "$log"

# A step reporting errors must not also trip the generic ERR trap. That
# would bury run_project_installer's own explanation under a misleading
# "just retry" message, and still needs to fail without marking it complete.
: >"$log"
printf 'phase=reboot_pending\ntarget_kernel=%s\n' "$target" >"$fake_state/state"
failure_output="$work/failure-output.log"
if printf 'n\n' |
	env \
		HOME="$fake_home" \
		XDG_RUNTIME_DIR="$work" \
		PATH="$fake_bin:/usr/bin:/bin" \
		KAIT2EN_STATE_DIR="$fake_state" \
		KAIT2EN_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_TARGET="$target" \
		KAIT2EN_TEST_REPOSITORY="$fake_repo" \
		KAIT2EN_TEST_ORIGIN_URL_FILE="$origin_url_file" \
		KAIT2EN_TEST_LOG="$log" \
		KAIT2EN_TEST_INSTALL_FAILS=1 \
		bash "$launcher" >"$failure_output" 2>&1; then
	printf 'error: kait2en-install returned success after a reported install failure\n' >&2
	exit 1
fi
grep -Fq 'reported errors' "$failure_output"
grep -Fq 'not marked complete' "$failure_output"
! grep -Fq 'installation stopped at line' "$failure_output"
! grep -Fq 'kait2en-prepare --complete' "$log"
