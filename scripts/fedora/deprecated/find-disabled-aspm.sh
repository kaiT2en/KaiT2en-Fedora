#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
# Lists PCIe devices that lack ASPM capability, have ASPM disabled,
# or lack/disable L1 PM Substates.
# Run with sudo for full lspci details.

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Please run as root (sudo $0)" >&2
    exit 1
fi

LSPCI_OUT=$(lspci -vv)

echo "=== Devices without any ASPM capability (no PCIe link at all, or ASPM not offered) ==="
echo "$LSPCI_OUT" | awk '
    /^[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-9]/ {
        if (dev != "" && has_lnkcap == 1 && has_aspm_cap == 0) {
            print dev
        }
        dev = $0
        has_lnkcap = 0
        has_aspm_cap = 0
    }
    /LnkCap:/ {
        has_lnkcap = 1
        if ($0 ~ /ASPM/) has_aspm_cap = 1
    }
    END {
        if (dev != "" && has_lnkcap == 1 && has_aspm_cap == 0) {
            print dev
        }
    }
'

echo
echo "=== Devices with ASPM capability but currently disabled (LnkCtl: ASPM Disabled) ==="
echo "$LSPCI_OUT" | awk '
    /^[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-9]/ { dev = $0 }
    /LnkCtl:.*ASPM Disabled/ { print dev }
'

echo
echo "=== Devices without L1 PM Substates capability (no [xxx v1] L1 PM Substates line) ==="
echo "$LSPCI_OUT" | awk '
    /^[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-9]/ {
        if (dev != "" && has_lnkcap == 1 && has_substates == 0) {
            print dev
        }
        dev = $0
        has_lnkcap = 0
        has_substates = 0
    }
    /LnkCap:/ { has_lnkcap = 1 }
    /L1 PM Substates/ { has_substates = 1 }
    END {
        if (dev != "" && has_lnkcap == 1 && has_substates == 0) {
            print dev
        }
    }
'

echo
echo "=== Devices with L1 PM Substates capability but all four sub-states disabled ==="
echo "$LSPCI_OUT" | awk '
    /^[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-9]/ { dev = $0 }
    /L1SubCtl1:/ {
        if ($0 !~ /PCI-PM_L1\.2\+/ && $0 !~ /PCI-PM_L1\.1\+/ && $0 !~ /ASPM_L1\.2\+/ && $0 !~ /ASPM_L1\.1\+/) {
            print dev
        }
    }
'