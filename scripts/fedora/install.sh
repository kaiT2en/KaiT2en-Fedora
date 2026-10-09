#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_min_kernel 7 0

# Asked before anything runs so the rest of the installation is unattended.
# KAIT2EN_INSTALL_TOUCHBAR=1|0 answers it ahead of time. Without a terminal
# the previous default (install) is kept.
ask_touchbar() {
	local answer
	if ! has_touch_bar; then
		KAIT2EN_INSTALL_TOUCHBAR=0
		return
	fi
	case "${KAIT2EN_INSTALL_TOUCHBAR:-}" in
		1 | 0) return ;;
	esac
	if [[ ! -t 0 ]]; then
		KAIT2EN_INSTALL_TOUCHBAR=1
		return
	fi
	cat <<'TEXT'

T2 Touch Bar (optional)

  T2 Touch Bar replaces Apple's built-in Touch Bar row with its own
  dark-first display. The bar stays black until you touch it or press Fn,
  learns how long to stay lit, offers a media row and an F-key row (hold Fn
  to switch), shows a fingerprint prompt for Touch ID and gives haptic
  feedback on key presses. It saves power because the bar is off most of
  the time. Multitouch is supported. Two-finger swipe on a dark bar changes
  volume, two-finger swipe on a lit bar switches the row. Three-finger
  swipe on a dark bar changes display brightness.

  Without it, the Touch Bar keeps working. Apple's native firmware row with
  esc, brightness, volume and media keys (F-keys while Fn is held) stays
  active.

TEXT
	read -r -p "Install kait2en-touchbar? [Y/n] " answer || answer=
	case "${answer,,}" in
		n | no) KAIT2EN_INSTALL_TOUCHBAR=0 ;;
		*) KAIT2EN_INSTALL_TOUCHBAR=1 ;;
	esac
}
ask_touchbar
export KAIT2EN_INSTALL_TOUCHBAR

install_multicall() {
	install -d -o root -g root -m 0755 /usr/local/bin
	install -o root -g root -m 0755 "$SCRIPT_DIR/kait2en-multicall" /usr/local/bin/kait2en-multicall
	ln -sfn kait2en-multicall /usr/local/bin/edit-grub
	ln -sfn kait2en-multicall /usr/local/bin/update-grub
}
run_step "install command helpers" install_multicall

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
	install-rtc-sync.sh
	install-apps.sh
	install-t2-services-common.sh
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

	run_step "$step" bash "$SCRIPT_DIR/$step" "${step_args[@]}"
	if (( STEP_STATUS != 0 )); then
		failed_steps+=("$step")
	fi
done

if (( ${#failed_steps[@]} > 0 )); then
	warn "installation completed with errors in: ${failed_steps[*]}"
fi
info "All Kait2en installation steps have been attempted"
info "reboot after reviewing the output"
