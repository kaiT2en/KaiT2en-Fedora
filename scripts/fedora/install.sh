#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_min_kernel 7 0

STEPS=(
	install-dependencies.sh
	install-kernel-args.sh
	install-dkms-modules.sh
	install-alsa-ucm.sh
	install-networkmanager-rules.sh
	install-t2-ncm-debug-service.sh
	rebuild-initramfs.sh
	install-suspend-service.sh
	install-apps.sh
)

for step in "${STEPS[@]}"; do
	info "running $step"
	bash "$SCRIPT_DIR/$step"
done

info "Kait2en installation completed"
info "reboot after reviewing the output"
