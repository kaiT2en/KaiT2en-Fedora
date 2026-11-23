#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# The installer builds every Rust project into one shared target directory.
# Cargo reuses a compiled crate only when all lockfiles pin the same version,
# so a semver-compatible version drift between projects compiles it twice.
# Fix a failure by running `cargo update` in every listed project.

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd "$repo_root"

# Projects the installer builds carry a Makefile next to their lockfile.
lockfiles=()
for makefile in apps/*/Makefile t2-services/*/Makefile; do
	[[ -f "${makefile%/Makefile}/Cargo.lock" ]] && lockfiles+=("${makefile%/Makefile}/Cargo.lock")
done

drift=$(awk '
	FNR == 1 { project = FILENAME; sub(/\/Cargo\.lock$/, "", project); name = "" }
	/^\[\[package\]\]/ { name = "" }
	/^name = / { name = $3; gsub(/"/, "", name) }
	/^version = / && name != "" {
		version = $3
		gsub(/"/, "", version)
		split(version, part, ".")
		line = part[1] == "0" ? part[1] "." part[2] : part[1]
		key = name " " line
		if (!((key, version) in seen)) {
			seen[key, version] = 1
			count[key]++
		}
		users[key] = users[key] "\n  " version " " project
	}
	END {
		for (key in count)
			if (count[key] > 1)
				print key ":" users[key]
	}
' "${lockfiles[@]}")

if [[ -n "$drift" ]]; then
	printf 'Cargo lockfiles pin different compatible versions:\n%s\n' "$drift" >&2
	printf 'Run cargo update in every Rust project to align them.\n' >&2
	exit 1
fi
printf 'Cargo lockfiles are aligned.\n'
