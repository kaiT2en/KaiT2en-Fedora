#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_fedora
require_command install mktemp rm udevadm

readonly RULE_DIR="/etc/udev/rules.d"
readonly RULE_FILE="$RULE_DIR/90-kait2en-t2-network.rules"

info "installing NetworkManager exclusion for the internal T2 NCM interface"

install -d -m 0755 "$RULE_DIR"

RULE_TMP="$(mktemp)"
trap 'rm -f "$RULE_TMP"' EXIT

cat >"$RULE_TMP" <<'EOF'
# Apple T2 internal CDC-NCM interface.
#
# Do not match by MAC address or interface name:
# both may differ between systems or naming-scheme versions.
#
# 05ac:8233 = Apple T2 Controller
# cdc_ncm   = internal USB CDC-NCM network function
ACTION=="add", SUBSYSTEM=="net", ENV{ID_BUS}=="usb", ENV{ID_VENDOR_ID}=="05ac", ENV{ID_MODEL_ID}=="8233", DRIVERS=="cdc_ncm", NAME:="t2_ncm", ENV{NM_UNMANAGED}="1", TAG+="systemd", ENV{SYSTEMD_WANTS}+="kait2en-t2-ncm-down.service"
# Renaming eth0 to t2_ncm emits a move event; retain the final properties.
ACTION=="change|move", SUBSYSTEM=="net", KERNEL=="t2_ncm", ENV{NM_UNMANAGED}="1", TAG+="systemd", ENV{SYSTEMD_WANTS}+="kait2en-t2-ncm-down.service"
EOF

install -o root -g root -m 0644 "$RULE_TMP" "$RULE_FILE"

udevadm control --reload-rules

info "installed $RULE_FILE"
info "the internal T2 debug network interface will be renamed to t2_ncm and unmanaged after reboot"
