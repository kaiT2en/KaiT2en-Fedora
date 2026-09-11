#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_min_kernel 7 0

install -d -o root -g root -m 0755 /usr/local/bin
install -o root -g root -m 0755 "$SCRIPT_DIR/kait2en-multicall" /usr/local/bin/kait2en-multicall
ln -sfn kait2en-multicall /usr/local/bin/edit-grub
ln -sfn kait2en-multicall /usr/local/bin/update-grub

STEPS=(
	install-dependencies.sh
	install-kernel-args.sh
	install-dkms-modules.sh
	install-gpu-runtime-pm.sh
	install-alsa-ucm.sh
	install-dsp.sh
	install-acpi-fixes.sh
	install-plymouth-theme.sh
	install-gdm-branding.sh
	install-suspend-service.sh
	install-apps.sh
	install-t2-remote.sh
	install-touchid.sh
	rebuild-initramfs.sh
)

failed_steps=()
for step in "${STEPS[@]}"; do
	info "running $step"
	if [[ "$step" == install-plymouth-theme.sh ]]; then
		step_args=(--defer-initramfs)
	elif [[ "$step" == install-gpu-runtime-pm.sh ]]; then
		step_args=(install --defer-initramfs)
	else
		step_args=()
	fi

	if ! bash "$SCRIPT_DIR/$step" "${step_args[@]}"; then
		warn "$step failed; continuing with the remaining installation steps"
		failed_steps+=("$step")
	fi
done

if (( ${#failed_steps[@]} > 0 )); then
	warn "installation completed with errors in: ${failed_steps[*]}"
fi
info "Kait2en installation completed"
info "reboot after reviewing the output"
