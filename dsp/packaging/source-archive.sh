#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
set -Eeuo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
if [[ $# -ne 1 || -e "$1" ]]; then
    printf 'Usage: %s NEW-ARCHIVE.tar.gz (must not exist)\n' "$0" >&2
    exit 1
fi
tar -C "$repo" --sort=name --mtime="@${SOURCE_DATE_EPOCH:-1789581600}" \
    --owner=0 --group=0 --numeric-owner \
    --exclude=dsp/build --exclude='__pycache__' \
    --transform='s,^,t2-dsp-0.1.0/,' \
    -cf - LICENSE LICENSING.md LICENSES dsp packaging/lifecycle | gzip -n >"$1"
