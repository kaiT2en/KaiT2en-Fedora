#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora

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

install_rust_app() {
	local path=$1 name=$2 target_user
	info "building and installing $name"

	target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] ||
		fail "$name must be built for the user who invoked sudo"

	if ! sudo -H -u "$target_user" make -C "$path" build; then
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
			if ! "$REPO_ROOT/apps/t2-hybrid-gpu-control/install.sh"; then
				warn "t2-hybrid-gpu-control installation failed; continuing"
			fi
			;;
		MacBookPro15,3)
			if ! make -C "$REPO_ROOT/apps/t2-hybrid-gpu-control" uninstall; then
				warn "unable to remove the inactive t2-hybrid-gpu-control app; continuing"
			fi
			if ! "$REPO_ROOT/apps/t2-dgpu-control/install.sh"; then
				warn "t2-dgpu-control installation failed; continuing"
			fi
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
}

install_app_assets() {
	install -d -o root -g root -m 0755 /usr/local/share/kait2en
	install -o root -g root -m 0644 \
		"$REPO_ROOT/assets/kait2en-app-logo.png" \
		/usr/local/share/kait2en/kait2en-wordmark.png
	install_kait2en_fonts
}
run_step "install app assets" install_app_assets
run_step "remove obsolete apps" remove_obsolete_apps
run_step "t2-fan-control" install_rust_app "$REPO_ROOT/apps/t2-fan-control" "t2-fan-control"
run_step "t2-smc-control" install_rust_app "$REPO_ROOT/apps/t2-smc-control" "t2-smc-control"
run_step "t2-power-explorer" install_rust_app "$REPO_ROOT/apps/t2-power-explorer" "t2-power-explorer"
run_step "t2-force-click" install_rust_app "$REPO_ROOT/apps/t2-force-click" "t2-force-click"
run_step "t2-journal" install_rust_app "$REPO_ROOT/t2-services/t2-journal" "t2-journal"
run_step "GPU control" install_gpu_control
if ! "$REPO_ROOT/apps/t2-cpu-control/install.sh"; then
	warn "t2-cpu-control installation failed; continuing"
fi
if ! "$REPO_ROOT/apps/t2-kernel-builder/install.sh"; then
	warn "t2-kernel-builder installation failed; continuing because it is optional"
	warn "retry it later with: sudo $REPO_ROOT/apps/t2-kernel-builder/install.sh"
fi
if ! "$REPO_ROOT/apps/t2-power-tune/install.sh"; then
	warn "t2-power-tune installation failed; continuing"
fi

info "apps installed"
