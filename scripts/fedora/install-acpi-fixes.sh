#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

status=0
log=$(mktemp)
trap 'rm -f "$log"' EXIT
python3 "$SCRIPT_DIR/acpi-autofix.py" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}

if (( status >= 128 )); then
	exit "$status"
fi

if (( status != 0 )); then
	reason=$(grep -E "could not be applied:" "$log" | tail -1)
	reason=${reason##*could not be applied: }
	warn "ACPI autofix failed: ${reason:-see the installation output above}"
fi

exit 0
