#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
template="$repo_root/auto-installer/macos-release-bootstrap.sh.in"
work=$(mktemp -d "${TMPDIR:-/tmp}/kait2en-bootstrap-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

release="$work/release"
payload="$work/payload"
fake_bin="$work/bin"
result="$work/result"
artifact=KaiT2en-Test-Installer
mkdir -p "$release" "$payload/scripts/macos" "$fake_bin"

cat >"$payload/scripts/macos/prepare-fedora-installer.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$KAIT2EN_TEST_RESULT"
EOF
(
	cd "$payload"
	zip -qr "$release/$artifact.zip" .
)
(
	cd "$release"
	sha256sum "$artifact.zip" >"$artifact.zip.sha256"
)

cat >"$fake_bin/uname" <<'EOF'
#!/usr/bin/env bash
printf 'Darwin\n'
EOF
cat >"$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
output=
url=
while (($# > 0)); do
	case "$1" in
		--output) output=$2; shift 2 ;;
		http*) url=$1; shift ;;
		*) shift ;;
	esac
done
cp "$KAIT2EN_TEST_RELEASE/${url##*/}" "$output"
EOF
cat >"$fake_bin/ditto" <<'EOF'
#!/usr/bin/env bash
unzip -q "$3" -d "$4"
EOF
cat >"$fake_bin/shasum" <<'EOF'
#!/usr/bin/env bash
[[ "$1 $2" == '-a 256' ]]
shift 2
sha256sum "$@"
EOF
chmod 0755 "$fake_bin"/*

sed \
	-e 's|@GITHUB_REPOSITORY@|example/KaiT2en-Fedora|g' \
	-e "s|@ARTIFACT_BASENAME@|$artifact|g" \
	"$template" >"$work/bootstrap.sh"

PATH="$fake_bin:/usr/bin:/bin" \
	KAIT2EN_RELEASE_BASE_URL=https://example.invalid/release \
	KAIT2EN_TEST_RELEASE="$release" \
	KAIT2EN_TEST_RESULT="$result" \
	KAIT2EN_TTY=/dev/null \
	bash "$work/bootstrap.sh" --edition kde >/dev/null

grep -Fxq -- '--edition kde' "$result"
