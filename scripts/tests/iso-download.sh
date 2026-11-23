#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
helper="$repo_root/scripts/macos/download-fedora-iso.sh"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-iso-download-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT
fake_bin="$test_root/bin"
mkdir -p "$fake_bin"

cat >"$fake_bin/stat" <<'EOF'
#!/usr/bin/env bash
[[ $1 == -f && $2 == %z ]]
wc -c <"$3" | tr -d ' '
EOF

cat >"$fake_bin/shasum" <<'EOF'
#!/usr/bin/env bash
[[ $1 == -a && $2 == 256 ]]
/usr/bin/sha256sum "$3"
EOF

cat >"$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -u
output=
resume=0
url=
while (($# > 0)); do
	case "$1" in
		--output) output=$2; shift 2 ;;
		--continue-at) resume=1; shift 2 ;;
		--retry|--retry-delay|--connect-timeout|--speed-limit|--speed-time|--max-time)
			shift 2 ;;
		--fail|--location) shift ;;
		*) url=$1; shift ;;
	esac
done
printf '%s\t%s\n' "$resume" "$url" >>"$CURL_LOG"

case "$url" in
	https://mirrors.fedoraproject.org/mirrorlist\?*)
		[[ "$SCENARIO" != mirrorlist_down ]] || exit 6
		case "$SCENARIO" in
			mirror_success)
				printf '%s\n' \
					https://mirror-one.example/Fedora-Test.iso \
					https://mirror-two.example/Fedora-Test.iso >"$output" ;;
			archive|oversize|local_write)
				printf '%s\n' https://mirror-one.example/Fedora-Test.iso >"$output" ;;
			cross_resume|bad_resume|no_range|all_fail)
				printf '%s\n' \
					https://mirror-one.example/Fedora-Test.iso \
					https://mirror-two.example/Fedora-Test.iso >"$output" ;;
			bad_verify)
				printf '%s\n' \
					https://mirror-one.example/Fedora-Test.iso \
					https://mirror-two.example/Fedora-Test.iso >"$output" ;;
			*) : >"$output" ;;
		esac
		exit 0
		;;
esac

case "$SCENARIO:$url" in
	mirror_success:https://mirror-one.example/*) exit 22 ;;
	mirror_success:https://mirror-two.example/*) printf correct-iso >"$output" ;;
	mirrorlist_down:https://download.fedoraproject.org/*) printf correct-iso >"$output" ;;
	archive:https://mirror-one.example/*|archive:https://download.fedoraproject.org/*) exit 22 ;;
	archive:https://archives.fedoraproject.org/*) printf correct-iso >"$output" ;;
	cross_resume:https://mirror-one.example/*)
		printf correct- >"$output"; exit 18 ;;
	cross_resume:https://mirror-two.example/*)
		[[ $resume -eq 1 && $(<"$output") == correct- ]] || exit 99
		printf iso >>"$output" ;;
	bad_resume:https://mirror-one.example/*)
		printf correct- >"$output"; exit 36 ;;
	bad_resume:https://mirror-two.example/*)
		[[ $resume -eq 1 && $(<"$output") == correct- ]] || exit 96
		printf iso >>"$output" ;;
	no_range:https://mirror-one.example/*)
		printf stale >"$output"; exit 18 ;;
	no_range:https://mirror-two.example/*)
		if [[ $resume -eq 1 ]]; then exit 33; fi
		[[ ! -e "$output" ]] || exit 98
		printf correct-iso >"$output" ;;
	bad_verify:https://mirror-one.example/*) printf short >"$output" ;;
	bad_verify:https://mirror-two.example/*) printf corrupt-iso >"$output" ;;
	bad_verify:https://download.fedoraproject.org/*) printf correct-iso >"$output" ;;
	oversize:https://mirror-one.example/*)
		[[ ! -e "$output" ]] || exit 97
		printf correct-iso >"$output" ;;
	local_write:https://mirror-one.example/*) exit 23 ;;
	all_fail:https://mirror-one.example/*)
		printf resumable >"$output"; exit 18 ;;
	all_fail:*) exit 22 ;;
	*) exit 22 ;;
esac
EOF
chmod +x "$fake_bin/curl" "$fake_bin/stat" "$fake_bin/shasum"

# shellcheck disable=SC1090
source "$helper"
canonical=https://download.fedoraproject.org/pub/fedora/linux/releases/44/Test/x86_64/iso/Fedora-Test.iso
payload=correct-iso
expected_size=${#payload}
expected_sha=$(printf '%s' "$payload" | /usr/bin/sha256sum | awk '{print $1}')

run_case() {
	local scenario=$1 destination
	destination="$test_root/$scenario/Fedora-Test.iso"
	mkdir -p "${destination%/*}"
	SCENARIO=$scenario CURL_LOG="$test_root/$scenario/curl.log" PATH="$fake_bin:$PATH" \
		export SCENARIO CURL_LOG PATH
	kait2en_download_iso "$destination" "$canonical" "$expected_size" "$expected_sha"
	cmp <(printf '%s' "$payload") "$destination"
}

run_case mirror_success
grep -Fq 'https://mirror-one.example/' "$test_root/mirror_success/curl.log"
grep -Fq 'https://mirror-two.example/' "$test_root/mirror_success/curl.log"

run_case mirrorlist_down
grep -Fq "$canonical" "$test_root/mirrorlist_down/curl.log"

run_case archive
grep -Fq 'https://archives.fedoraproject.org/pub/archive/' "$test_root/archive/curl.log"

run_case cross_resume
grep -Fq $'1\thttps://mirror-two.example/' "$test_root/cross_resume/curl.log"

run_case bad_resume
grep -Fq $'1\thttps://mirror-two.example/' "$test_root/bad_resume/curl.log"

run_case no_range
[[ $(grep -Fc 'https://mirror-two.example/' "$test_root/no_range/curl.log") -eq 2 ]]
grep -Fq $'0\thttps://mirror-two.example/' "$test_root/no_range/curl.log"

run_case bad_verify
grep -Fq "$canonical" "$test_root/bad_verify/curl.log"

oversize_dir="$test_root/oversize"
mkdir -p "$oversize_dir"
printf 'far-too-long-partial' >"$oversize_dir/Fedora-Test.iso.part"
run_case oversize
[[ ! -e "$oversize_dir/Fedora-Test.iso.part" ]]

write_dir="$test_root/local_write"
mkdir -p "$write_dir"
SCENARIO=local_write CURL_LOG="$write_dir/curl.log" PATH="$fake_bin:$PATH" \
	export SCENARIO CURL_LOG PATH
if kait2en_download_iso "$write_dir/Fedora-Test.iso" "$canonical" \
	"$expected_size" "$expected_sha" 2>"$write_dir/error"; then
	printf 'local write failure unexpectedly succeeded\n' >&2
	exit 1
fi
[[ $(grep -Fc 'https://mirror-one.example/' "$write_dir/curl.log") -eq 1 ]]
! grep -Fq $'\thttps://download.fedoraproject.org/' "$write_dir/curl.log"
grep -Fq 'check free space' "$write_dir/error"

cache_dir="$test_root/cache"
mkdir -p "$cache_dir"
printf '%s' "$payload" >"$cache_dir/Fedora-Test.iso"
SCENARIO=all_fail CURL_LOG="$cache_dir/curl.log" PATH="$fake_bin:$PATH" \
	export SCENARIO CURL_LOG PATH
kait2en_download_iso "$cache_dir/Fedora-Test.iso" "$canonical" \
	"$expected_size" "$expected_sha"
[[ ! -e "$cache_dir/curl.log" ]]

failure_dir="$test_root/all_fail"
mkdir -p "$failure_dir"
SCENARIO=all_fail CURL_LOG="$failure_dir/curl.log" PATH="$fake_bin:$PATH" \
	export SCENARIO CURL_LOG PATH
if kait2en_download_iso "$failure_dir/Fedora-Test.iso" "$canonical" \
	"$expected_size" "$expected_sha" 2>"$failure_dir/error"; then
	printf 'all-failure ISO download unexpectedly succeeded\n' >&2
	exit 1
fi
[[ -f "$failure_dir/Fedora-Test.iso.part" ]]
grep -Fq 'resumable partial file' "$failure_dir/error"
grep -Fq 'the target disk was not changed' \
	"$repo_root/scripts/macos/prepare-fedora-installer.sh"
download_line=$(grep -n 'kait2en_download_iso "$ISO_PATH"' \
	"$repo_root/scripts/macos/prepare-fedora-installer.sh" | cut -d: -f1)
disk_write_line=$(grep -n '^DISK_TOUCHED=1' \
	"$repo_root/scripts/macos/prepare-fedora-installer.sh" | cut -d: -f1)
((download_line < disk_write_line))

if kait2en_fedora_release_path 'https://evil.example/releases/44/Test.iso' >/dev/null; then
	printf 'unsafe Fedora host was accepted\n' >&2
	exit 1
fi
if kait2en_fedora_release_path \
	'https://download.fedoraproject.org/pub/fedora/linux/releases/44/../Test.iso' >/dev/null; then
	printf 'unsafe Fedora path was accepted\n' >&2
	exit 1
fi

printf 'Fedora ISO mirror failover tests passed.\n'
