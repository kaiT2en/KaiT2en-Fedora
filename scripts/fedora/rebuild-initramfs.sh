#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dracut

KVER="$(kernel_release)"
ADD_DRIVERS="t2gmux i915"

info "rebuilding initramfs for $KVER"
dracut --force --add-drivers "$ADD_DRIVERS" "/boot/initramfs-$KVER.img" "$KVER"

info "initramfs rebuilt"
