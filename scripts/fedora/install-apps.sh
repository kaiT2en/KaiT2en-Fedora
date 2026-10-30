#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora

# Without arguments every app is installed. kait2en-update names only the
# apps whose sources changed, for example: install-apps.sh t2-fan-control
SELECTED_APPS=("$@")
[[ ${#SELECTED_APPS[@]} -gt 0 || -z "${KAIT2EN_APPS:-}" ]] ||
	IFS=, read -r -a SELECTED_APPS <<<"$KAIT2EN_APPS"

wanted() {
	local app
	(( ${#SELECTED_APPS[@]} == 0 )) && return 0
	for app in "${SELECTED_APPS[@]}"; do
		[[ "$app" == "$1" ]] && return 0
	done
	return 1
}

OBSOLETE_UNITS=(
	kait2en-t2-smc-charge-limit.service
)

remove_obsolete_apps() {
	local unit reload=0

	info "removing obsolete t2-gpu-switch installation"
	rm -f \
		/usr/local/bin/t2-gpu-switch \
		/usr/local/libexec/t2-gpu-switch-helper \
		/usr/local/share/applications/org.t2gpuswitch.gtk.desktop

	for unit in "${OBSOLETE_UNITS[@]}"; do
		if systemctl list-unit-files "$unit" &>/dev/null; then
			info "removing obsolete $unit"
			systemctl disable --now "$unit" || warn "could not disable $unit"
			reload=1
		fi
		[[ -e "/usr/local/lib/systemd/system/$unit" ]] || continue
		rm -f "/usr/local/lib/systemd/system/$unit"
		reload=1
	done
	[[ "$reload" -eq 0 ]] || systemctl daemon-reload
}

# t2-zone-power replaces t2-cpu-control. Its helper restores the firmware
# power limits, no_turbo, TCC target, maximum frequency and thermald state it
# captured at boot. Every step is skipped when there is nothing left to do,
# so a removed or half removed installation is handled as well.
remove_t2_cpu_control() {
	local helper=/usr/local/libexec/t2-cpu-control-helper path found=0
	local paths=(
		/usr/local/bin/t2-cpu-control
		"$helper"
		/usr/local/libexec/t2-cpu-control-status
		/usr/local/libexec/t2-cpu-kernel-benchmark
		/usr/local/lib/systemd/system/t2-cpu-control.service
		/usr/local/lib/systemd/system-sleep/t2-cpu-control
		/usr/local/share/applications/org.t2cpucontrol.gtk.desktop
		/usr/local/share/icons/hicolor/scalable/apps/org.t2cpucontrol.gtk.svg
		/usr/share/polkit-1/actions/org.t2cpucontrol.policy
		/etc/t2-cpu-control.conf
		/var/lib/t2-cpu-control
		/run/t2-cpu-control
	)

	for path in "${paths[@]}"; do
		[[ -e "$path" ]] && found=1
	done
	[[ "$found" -eq 1 ]] || return 0
	info "removing t2-cpu-control, replaced by t2-zone-power"

	if [[ -x "$helper" && -r /run/t2-cpu-control/firmware-defaults ]]; then
		"$helper" restore || warn "could not restore the firmware CPU limits, they return on the next boot"
	fi
	if systemctl list-unit-files t2-cpu-control.service &>/dev/null; then
		systemctl disable --now t2-cpu-control.service || warn "could not disable t2-cpu-control.service"
	fi
	rm -rf "${paths[@]}"
	systemctl daemon-reload
	gtk-update-icon-cache --force --ignore-theme-index /usr/local/share/icons/hicolor ||
		warn "could not refresh the icon cache"
}

# Rust projects used to build into their own target directory, and some
# installers never cleaned it. They build into the shared one now.
remove_project_cargo_targets() {
	local dir
	for dir in "$REPO_ROOT"/apps/*/target "$REPO_ROOT"/t2-services/*/target; do
		[[ -f "$dir/CACHEDIR.TAG" ]] || continue
		info "removing old Cargo build directory $dir"
		rm -rf "$dir"
	done
}

install_rust_app() {
	local path=$1 name=$2 target_user
	info "building and installing $name"

	target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] ||
		fail "$name must be built for the user who invoked sudo"

	if ! build_cargo_project "$path" "$target_user"; then
		warn "$name build failed; skipping this app and continuing"
		return 0
	fi
	if ! make -C "$path" install; then
		warn "$name installation failed; continuing with the remaining apps"
		return 0
	fi
	if [[ "$name" == t2-journal ]]; then
		python3 "$REPO_ROOT/packaging/lifecycle/kait2en-lifecycle.py" t2-journal record-source || warn "could not record t2-journal installation ownership"
	fi
	mark_applied "app:$name"
}

install_touchbar() {
	"$REPO_ROOT/apps/t2-touchbar/install.sh"
	mark_applied app:t2-touchbar
}

# Apps with their own install.sh. A failure is reported and the rest continues.
install_script_app() {
	local name=$1
	if "$REPO_ROOT/apps/$name/install.sh"; then
		mark_applied "app:$name"
		return 0
	fi
	warn "$name installation failed; continuing"
	return 1
}

install_gpu_control() {
	local model

	[[ -r /sys/class/dmi/id/product_name ]] || {
		info "DMI product name not found, skipping GPU control"
		return
	}
	read -r model </sys/class/dmi/id/product_name

	case "$model" in
		MacBookPro15,1|MacBookPro16,1|MacBookPro16,4)
			info "installing hybrid graphics support for $model"
			if ! make -C "$REPO_ROOT/apps/t2-dgpu-control" uninstall; then
				warn "unable to remove the inactive t2-dgpu-control app; continuing"
			fi
			install_script_app t2-hybrid-gpu-control || return 0
			;;
		MacBookPro15,3)
			if ! make -C "$REPO_ROOT/apps/t2-hybrid-gpu-control" uninstall; then
				warn "unable to remove the inactive t2-hybrid-gpu-control app; continuing"
			fi
			install_script_app t2-dgpu-control || return 0
			;;
		*)
			info "Model $model has no supported switchable AMD dGPU"
			if ! make -C "$REPO_ROOT/apps/t2-hybrid-gpu-control" uninstall; then
				warn "unable to remove t2-hybrid-gpu-control; continuing"
			fi
			if ! make -C "$REPO_ROOT/apps/t2-dgpu-control" uninstall; then
				warn "unable to remove t2-dgpu-control; continuing"
			fi
			;;
	esac
	mark_applied app:gpu-control
}

install_app_assets() {
	install -d -o root -g root -m 0755 /usr/local/share/kait2en
	install -o root -g root -m 0644 \
		"$REPO_ROOT/assets/kait2en-app-logo.png" \
		/usr/local/share/kait2en/kait2en-wordmark.png
	install -o root -g root -m 0644 \
		"$REPO_ROOT/assets/kait2en-app-logo-light.png" \
		/usr/local/share/kait2en/kait2en-wordmark-light.png
	install_kait2en_fonts
}
run_step "install app assets" install_app_assets
(( STEP_STATUS == 0 )) && mark_applied app:assets
run_step "remove obsolete apps" remove_obsolete_apps
run_step "remove old Cargo build directories" remove_project_cargo_targets
for app in t2-fan-control t2-smc-control t2-power-explorer t2-force-click t2-debug t2-airshare t2-drop; do
	wanted "$app" && run_step "$app" install_rust_app "$REPO_ROOT/apps/$app" "$app"
done
if ! wanted t2-kbd-backlight; then
	:
elif has_keyboard_backlight; then
	run_step "t2-kbd-backlight" install_rust_app "$REPO_ROOT/apps/t2-kbd-backlight" "t2-kbd-backlight"
else
	info "no keyboard backlight on this model; skipping t2-kbd-backlight"
fi
if ! wanted t2-touchbar; then
	:
elif ! has_touch_bar; then
	info "no Touch Bar on this model; skipping kait2en-touchbar"
elif [[ "${KAIT2EN_INSTALL_TOUCHBAR:-1}" == 1 ]]; then
	run_step "kait2en-touchbar" install_touchbar
elif [[ -x /usr/local/bin/kait2en-touchbar ]]; then
	info "kait2en-touchbar not selected; the existing installation is left unchanged"
	info "remove it with: sudo $REPO_ROOT/apps/t2-touchbar/uninstall.sh"
else
	info "kait2en-touchbar not selected; the native Touch Bar row stays active"
fi
wanted t2-journal && run_step "t2-journal" install_rust_app "$REPO_ROOT/t2-services/t2-journal" "t2-journal"
wanted gpu-control && run_step "GPU control" install_gpu_control
run_step "remove t2-cpu-control" remove_t2_cpu_control
wanted t2-zone-power && run_step "t2-zone-power" install_rust_app "$REPO_ROOT/apps/t2-zone-power" "t2-zone-power"
if wanted kait2en-update; then
	run_step "kait2en-update" install_rust_app "$REPO_ROOT/apps/kait2en-update" "kait2en-update"
fi
if ! wanted t2-kernel-builder; then
	:
elif "$REPO_ROOT/apps/t2-kernel-builder/install.sh"; then
	mark_applied app:t2-kernel-builder
else
	warn "t2-kernel-builder installation failed; continuing because it is optional"
	warn "retry it later with: sudo $REPO_ROOT/apps/t2-kernel-builder/install.sh"
fi
wanted t2-power-tune && install_script_app t2-power-tune || true
wanted t2-sidecar && install_script_app t2-sidecar || true

info "apps installed"
