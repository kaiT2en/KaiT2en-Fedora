#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_min_kernel 7 0

# --only step,step runs just those steps, as kait2en-update does after it
# compared the installed state with GitHub. Optional parts then keep what is
# installed, no question is asked.
ONLY_STEPS=()
if [[ ${1:-} == --only ]]; then
	[[ -n ${2:-} ]] || fail "usage: install.sh [--only step,step,...]"
	IFS=, read -r -a ONLY_STEPS <<<"$2"
	if [[ -z "${KAIT2EN_INSTALL_TOUCHBAR:-}" ]]; then
		KAIT2EN_INSTALL_TOUCHBAR=0
		[[ -x /usr/local/bin/kait2en-touchbar ]] && KAIT2EN_INSTALL_TOUCHBAR=1
	fi
	KAIT2EN_INSTALL_VORNEHM=0
elif (( $# > 0 )); then
	fail "usage: install.sh [--only step,step,...]"
fi

selected() {
	local step
	(( ${#ONLY_STEPS[@]} == 0 )) && return 0
	for step in "${ONLY_STEPS[@]}"; do
		[[ "$step" == "$1" ]] && return 0
	done
	return 1
}
selected install-vornehm.sh && (( ${#ONLY_STEPS[@]} > 0 )) && KAIT2EN_INSTALL_VORNEHM=1

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

# KaiT2en Vornehm changes the GNOME desktop of the user who runs sudo, so
# it is only offered when GNOME is there. KAIT2EN_INSTALL_VORNEHM=1|0
# answers it ahead of time. Without a terminal nothing is changed.
VORNEHM_USER=${SUDO_USER:-}

# Runs a command as that user, inside the running session when there is one.
as_vornehm_user() {
	local uid home runtime
	uid=$(id -u "$VORNEHM_USER")
	home=$(getent passwd "$VORNEHM_USER" | cut -d: -f6)
	runtime=/run/user/$uid
	if [[ -S "$runtime/bus" ]]; then
		runuser -u "$VORNEHM_USER" -- env HOME="$home" XDG_RUNTIME_DIR="$runtime" \
			DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" "$@"
	else
		runuser -u "$VORNEHM_USER" -- env HOME="$home" dbus-run-session -- "$@"
	fi
}

# A shell setting that holds a list of extension UUIDs, one per line.
extension_setting() {
	as_vornehm_user gsettings get org.gnome.shell "$1" 2>/dev/null |
		tr -d "[]'," | tr ' ' '\n' | grep -v -e '^$' -e '^@as$' | sort || true
}

# Names of the extensions the user added and that really run. The running
# shell knows them. Without a session the remembered list is checked the
# way the shell does at login. What Fedora enables out of the box and the
# official GNOME extensions are never listed, so a fresh install shows none.
active_extensions() {
	local home uuids uuid dir name defaults
	home=$(getent passwd "$VORNEHM_USER" | cut -d: -f6)
	defaults=$(GSETTINGS_BACKEND=memory gsettings get org.gnome.shell enabled-extensions 2>/dev/null) || defaults=
	if ! uuids=$(as_vornehm_user gnome-extensions list --enabled 2>/dev/null); then
		[[ $(as_vornehm_user gsettings get org.gnome.shell disable-user-extensions 2>/dev/null) == true ]] &&
			return 0
		uuids=$(comm -23 <(extension_setting enabled-extensions) <(extension_setting disabled-extensions))
	fi
	for uuid in $uuids; do
		[[ $uuid == kait2en-* || $uuid == *@gnome-shell-extensions.gcampax.github.com ]] && continue
		[[ $defaults == *"'$uuid'"* ]] && continue
		for dir in "$home/.local/share/gnome-shell/extensions/$uuid" \
			"/usr/local/share/gnome-shell/extensions/$uuid" \
			"/usr/share/gnome-shell/extensions/$uuid"; do
			[[ -r $dir/metadata.json ]] || continue
			name=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1])).get("name", ""))' \
				"$dir/metadata.json" 2>/dev/null) || name=
			printf '    %s\n' "${name:-$uuid}"
			break
		done
	done
}

ask_vornehm() {
	local answer extensions
	if ! command -v gnome-shell >/dev/null 2>&1 || [[ -z "$VORNEHM_USER" || "$VORNEHM_USER" == root ]]; then
		KAIT2EN_INSTALL_VORNEHM=0
		return
	fi
	case "${KAIT2EN_INSTALL_VORNEHM:-}" in
		1 | 0) return ;;
	esac
	if [[ ! -t 0 ]]; then
		KAIT2EN_INSTALL_VORNEHM=0
		return
	fi
	cat <<'TEXT'

KaiT2en Vornehm (optional)

  KaiT2en Vornehm is a calm, dark look for GNOME. Darker surfaces with fine
  chrome rims, frosted glass behind menus, dialogs and notifications, a
  slim dock whose icons lift on hover, app icons on dark tiles, graphite
  folders and a blurred wallpaper in the overview. It switches GNOME to dark
  style and sets a BLACK wallpaper by Jean-Marc Denis. There is nothing to
  configure. Remove it at any time with install-vornehm.sh --remove, which
  brings back your previous wallpaper, style and icons.

  Without it, GNOME keeps its own look.

TEXT
	extensions=$(active_extensions | sort -f)
	if [[ -n "$extensions" ]]; then
		cat <<'TEXT'
  (Ignore on fresh KaiT2en installs) Extensions you added yourself
  can get in the way, especially ones that restyle the shell. These
  are currently active :

TEXT
		printf '%s\n\n' "$extensions"
		cat <<'TEXT'
  It is best to switch all of them off, try KaiT2en Vornehm and then turn
  them back on one by one, so you notice any side effect right away.

TEXT
	fi
	read -r -p "Install KaiT2en Vornehm for $VORNEHM_USER? [Y/n] " answer || answer=
	case "${answer,,}" in
		n | no) KAIT2EN_INSTALL_VORNEHM=0 ;;
		*) KAIT2EN_INSTALL_VORNEHM=1 ;;
	esac
}
ask_vornehm

install_vornehm() {
	as_vornehm_user bash "$SCRIPT_DIR/install-vornehm.sh"
}

install_multicall() {
	install -d -o root -g root -m 0755 /usr/local/bin
	install -o root -g root -m 0755 "$SCRIPT_DIR/kait2en-multicall" /usr/local/bin/kait2en-multicall
	ln -sfn kait2en-multicall /usr/local/bin/edit-grub
	ln -sfn kait2en-multicall /usr/local/bin/update-grub
	ln -sfn kait2en-multicall /usr/local/bin/boot-macos
	ln -sfn kait2en-multicall /usr/local/bin/update-modules
	ln -sfn kait2en-multicall /usr/local/bin/update-apps
}
if selected command-helpers; then
	run_step "install command helpers" install_multicall
	(( STEP_STATUS == 0 )) && mark_applied command-helpers
fi

STEPS=(
	install-dependencies.sh
	install-kernel-args.sh
	install-dkms-modules.sh
	install-kernel-source.sh
	install-macos-mount.sh
	install-gpu-runtime-pm.sh
	install-i915.sh
	install-wireless.sh
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

# Steps whose files end up in the initramfs. A selective run that includes
# one of them rebuilds the initramfs as well.
INITRAMFS_STEPS=(
	install-dkms-modules.sh
	install-gpu-runtime-pm.sh
	install-i915.sh
	install-wireless.sh
	install-acpi-fixes.sh
	install-plymouth-theme.sh
)
if (( ${#ONLY_STEPS[@]} > 0 )); then
	for step in "${INITRAMFS_STEPS[@]}"; do
		if selected "$step"; then
			ONLY_STEPS+=(rebuild-initramfs.sh)
			break
		fi
	done
fi

failed_steps=()
for step in "${STEPS[@]}"; do
	selected "$step" || continue
	info "running $step"
	if [[ "$step" == install-plymouth-theme.sh ]]; then
		step_args=(--defer-initramfs)
	elif [[ "$step" == install-gpu-runtime-pm.sh || "$step" == install-i915.sh ||
		"$step" == install-wireless.sh ]]; then
		step_args=(install --defer-initramfs)
	else
		step_args=()
	fi

	run_step "$step" bash "$SCRIPT_DIR/$step" "${step_args[@]}"
	if (( STEP_STATUS != 0 )); then
		failed_steps+=("$step")
	elif [[ "$step" != install-apps.sh ]]; then
		mark_applied "$step"
	fi
done

if [[ $KAIT2EN_INSTALL_VORNEHM == 1 ]]; then
	run_step "KaiT2en Vornehm for $VORNEHM_USER" install_vornehm
	if (( STEP_STATUS == 0 )); then
		mark_applied install-vornehm.sh
	else
		failed_steps+=(install-vornehm.sh)
	fi
elif (( ${#ONLY_STEPS[@]} == 0 )) && [[ -n "$VORNEHM_USER" && "$VORNEHM_USER" != root ]] && command -v gnome-shell >/dev/null 2>&1; then
	info "KaiT2en Vornehm not selected; add it later with: $SCRIPT_DIR/install-vornehm.sh"
fi

if (( ${#failed_steps[@]} > 0 )); then
	warn "installation completed with errors in: ${failed_steps[*]}"
fi
info "All Kait2en installation steps have been attempted"
info "reboot after reviewing the output"
