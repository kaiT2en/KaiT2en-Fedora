#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dracut

KVER="$(kernel_release)"
ADD_DRIVERS="t2smc t2bce t2bdrm t2hid t2touchbar_bl t2touchbar_kbd hid_t2magicmouse t2mfi_fastcharge t2gmux t2thunderbolt"

info "rebuilding initramfs for $KVER"
dracut --force --add-drivers "$ADD_DRIVERS" "/boot/initramfs-$KVER.img" "$KVER"

info "initramfs rebuilt"
