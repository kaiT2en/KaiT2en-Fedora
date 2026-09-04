#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

install_mode=all
case "${1:-}" in
	"") ;;
	--react-drm-only) install_mode=react-drm ;;
	*)
		printf 'usage: %s [--react-drm-only]\n' "${0##*/}" >&2
		exit 2
		;;
esac

require_root
require_repo_root
require_fedora
require_command \
	awk chown cut desktop-file-validate dnf env getent grep id install mktemp \
	modinfo npm rm rpm sleep sudo systemctl tar tr udevadm \
	update-desktop-database usermod
if [[ "$install_mode" == all ]]; then
	require_command cargo make
fi

REACT_DRM_FEDORA_PACKAGES=(
	nodejs22-bin
	nodejs22-npm-bin
	python3
	gcc
	gcc-c++
	make
	pkgconf-pkg-config
	systemd-devel
	libdrm-devel
	cairo-devel
	librsvg2-devel
	brightnessctl
	cava
)

REACT_DRM_FEDORA_NODE_PACKAGES=(
	nodejs
	nodejs-libs
	nodejs-npm
	nodejs-docs
	nodejs-full-i18n
)

REACT_DRM_CONFLICT_DAEMONS=(
	tiny-dfr
	mac-touchbar-plus
)

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
			systemctl disable --now "$unit" || true
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
}

systemd_escape_path() {
	local value=$1
	[[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] ||
		fail "paths containing line breaks are not supported"
	value=${value//\\/\\x5c}
	value=${value//$'\t'/\\x09}
	value=${value// /\\x20}
	value=${value//\"/\\x22}
	value=${value//\'/\\x27}
	value=${value//%/%%}
	printf '%s' "$value"
}

has_t2_touchbar_model() {
	local model
	[[ -r /sys/class/dmi/id/product_name ]] || {
		info "DMI product name not found, skipping react-drm"
		return 1
	}

	read -r model </sys/class/dmi/id/product_name
	case "$model" in
		MacBookPro15,1|MacBookPro15,2|MacBookPro15,3|MacBookPro15,4|\
		MacBookPro16,1|MacBookPro16,2|MacBookPro16,3|MacBookPro16,4)
			return 0
			;;
		*)
			info "Model $model has no T2 Touch Bar entry, skipping react-drm"
			return 1
			;;
	esac
}

install_gpu_control() {
	local model

	[[ -r /sys/class/dmi/id/product_name ]] || {
		info "DMI product name not found, skipping GPU control"
		return
	}
	read -r model </sys/class/dmi/id/product_name

	case "$model" in
		MacBookPro15,1)
			info "installing tested hybrid graphics support for $model"
			if ! make -C "$REPO_ROOT/apps/t2-dgpu-control" uninstall; then
				warn "unable to remove the inactive t2-dgpu-control app; continuing"
			fi
			if ! "$REPO_ROOT/apps/t2-hybrid-gpu-control/install.sh"; then
				warn "t2-hybrid-gpu-control installation failed; continuing"
			fi
			;;
		MacBookPro15,3|MacBookPro16,1|MacBookPro16,4)
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

install_react_drm() {
	local target_user target_home target_uid target_group src dst backup_dir relative
	local installed_node_packages=() package daemon unit group groups
	local missing_groups=()
	local service_dir service_file temporary_file env_q workdir_q start_q detach_q
	local app_dir launcher_file launcher_tmp
	local desktop extension_uuid extension_src extension_dst
	if ! has_t2_touchbar_model; then
		return
	fi
	for module in t2bdrm t2touchbar_bl; do
		modinfo "$module" >/dev/null 2>&1 ||
			fail "required KaiT2en kernel module is missing: $module"
	done

	target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] ||
		fail "react-drm must be installed for the user who invoked sudo"

	target_home="$(getent passwd "$target_user" | cut -d: -f6)"
	target_uid="$(id -u "$target_user")"
	target_group="$(id -gn "$target_user")"
	[[ -n "$target_home" && -d "$target_home" ]] ||
		fail "unable to determine home directory for $target_user"

	src="$REPO_ROOT/apps/react-drm"
	dst="$target_home/react-drm"
	for package in package.json package-lock.json .env.example.kait2en system/99-react-drm-kait2en.rules system/react-drm.service system/react-drm-tb-detach; do
		[[ -r "$src/$package" ]] || fail "react-drm deployment file is missing: $package"
	done
	extension_uuid="window-monitor-pro@muhammed.hussien2030.gmail.com"
	extension_src="$src/gnome-extension/window-monitor-pro"
	for package in extension.js metadata.json; do
		[[ -r "$extension_src/$package" ]] ||
			fail "react-drm GNOME extension file is missing: $package"
	done
	[[ -x "$src/system/react-drm-tb-detach" ]] ||
		fail "react-drm deployment helper is not executable: system/react-drm-tb-detach"
	if [[ -e "$dst" && ! -f "$dst/package.json" ]]; then
		fail "deployment directory exists but does not look like react-drm: $dst"
	fi

	[[ -S "/run/user/$target_uid/bus" ]] ||
		fail "user session bus not available for $target_user; run this installer from an active desktop login"

	run_as_target() {
		sudo -H -u "$target_user" env \
			XDG_RUNTIME_DIR="/run/user/$target_uid" \
			DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$target_uid/bus" \
			"$@"
	}

	desktop=$(
		run_as_target systemctl --user show-environment |
			awk -F= '$1 == "XDG_CURRENT_DESKTOP" { print tolower($2); exit }'
	)
	if [[ "$desktop" == *gnome* ]]; then
		require_command gnome-extensions gsettings
		extension_dst="$target_home/.local/share/gnome-shell/extensions/$extension_uuid"
		info "installing Window Monitor Pro for react-drm"
		install -d -o "$target_user" -g "$target_group" -m 0755 "$extension_dst"
		install -o "$target_user" -g "$target_group" -m 0644 \
			"$extension_src/extension.js" \
			"$extension_src/metadata.json" \
			"$extension_dst/"
		run_as_target gsettings set org.gnome.shell disable-user-extensions false
		if ! run_as_target gnome-extensions enable "$extension_uuid"; then
			info "Window Monitor Pro will be enabled after the next login"
		fi
	fi

	info "installing react-drm Fedora dependencies"
	for package in "${REACT_DRM_FEDORA_NODE_PACKAGES[@]}"; do
		if rpm -q "$package" >/dev/null 2>&1; then
			installed_node_packages+=("$package")
		fi
	done
	if [[ ${#installed_node_packages[@]} -gt 0 ]]; then
		dnf -y do \
			--action=remove "${installed_node_packages[@]}" \
			--action=install "${REACT_DRM_FEDORA_PACKAGES[@]}"
	else
		dnf install -y "${REACT_DRM_FEDORA_PACKAGES[@]}"
	fi

	info "removing conflicting Touch Bar daemons"
	for daemon in "${REACT_DRM_CONFLICT_DAEMONS[@]}"; do
		unit="${daemon}.service"
		if systemctl cat "$unit" >/dev/null 2>&1; then
			systemctl disable --now "$unit"
		fi
		if run_as_target systemctl --user cat "$unit" >/dev/null 2>&1; then
			run_as_target systemctl --user disable --now "$unit"
		fi
		if rpm -q "$daemon" >/dev/null 2>&1; then
			dnf remove -y "$daemon"
		fi
	done
	systemctl daemon-reload
	run_as_target systemctl --user daemon-reload

	for group in video input; do
		getent group "$group" >/dev/null || fail "required group does not exist: $group"
		if ! id -nG "$target_user" | tr ' ' '\n' | grep -Fxq "$group"; then
			missing_groups+=("$group")
		fi
	done
	if [[ ${#missing_groups[@]} -gt 0 ]]; then
		groups=$(IFS=,; printf '%s' "${missing_groups[*]}")
		info "adding $target_user to groups: ${missing_groups[*]}"
		usermod -aG "$groups" "$target_user"
		info "log out and back in after installation so group changes take effect"
	fi

	info "installing react-drm udev rules"
	install -d -o root -g root -m 0755 /etc/udev/rules.d
	rm -f /etc/udev/rules.d/99-react-drm-uinput.rules
	install -o root -g root -m 0644 \
		"$src/system/99-react-drm-kait2en.rules" \
		/etc/udev/rules.d/99-react-drm.rules
	udevadm control --reload
	udevadm trigger --action=add --subsystem-match=usb --subsystem-match=backlight
	udevadm trigger --action=add --subsystem-match=misc --sysname-match=uinput

	info "copying react-drm source to $dst"
	backup_dir=$(mktemp -d /tmp/react-drm-user-data.XXXXXX)
	for relative in \
		.env \
		linux-touchbar-control-center/config.ts \
		linux-touchbar-control-center/custom-layer.json
	do
		if [[ -f "$dst/$relative" ]]; then
			install -D -m 0644 "$dst/$relative" "$backup_dir/$relative"
		fi
	done
	rm -rf "$dst"
	install -d -o "$target_user" -g "$target_group" -m 0755 "$dst"
	tar -C "$src" \
		--exclude='.git' \
		--exclude='node_modules' \
		--exclude='dist' \
		--exclude='linux-touchbar-control-center/dist' \
		-cf - . | tar -C "$dst" -xf -
	if [[ -f "$backup_dir/.env" ]]; then
		install -m 0644 "$backup_dir/.env" "$dst/.env"
	else
		install -m 0644 "$src/.env.example.kait2en" "$dst/.env"
	fi
	# The first unified KaiT2en profile named only apple-panel-bl.  That hid
	# t2gmux's gmux_backlight as soon as react-drm started loading .env.  Migrate
	# only that exact shipped default and preserve every customized candidate
	# list unchanged.
	if grep -Fxq 'REACT_DRM_DISP_BACKLIGHT_NAMES=apple-panel-bl' "$dst/.env"; then
		sed -i \
			's/^REACT_DRM_DISP_BACKLIGHT_NAMES=apple-panel-bl$/REACT_DRM_DISP_BACKLIGHT_NAMES=apple-panel-bl,gmux_backlight,intel_backlight,acpi_video0/' \
			"$dst/.env"
	fi
	for relative in \
		linux-touchbar-control-center/config.ts \
		linux-touchbar-control-center/custom-layer.json
	do
		if [[ -f "$backup_dir/$relative" ]]; then
			install -D -m 0644 "$backup_dir/$relative" "$dst/$relative"
		fi
	done
	rm -rf "$backup_dir"
	chown -R "$target_user:$target_group" "$dst"

	info "building react-drm"
	run_as_target npm --prefix "$dst" ci
	run_as_target npm --prefix "$dst/linux-touchbar-control-center" run build
	run_as_target npm --prefix "$dst/config-gui" run build

	info "installing react-drm config editor"
	app_dir="$target_home/.local/share/applications"
	launcher_file="$app_dir/react-drm-config-gui.desktop"
	launcher_tmp=$(mktemp --suffix=.desktop /tmp/react-drm-config-gui.XXXXXX)
	awk -v electron="$dst/node_modules/.bin/electron" -v gui="$dst/config-gui" '
		/^Exec=/ { printf "Exec=\"%s\" \"%s\"\n", electron, gui; next }
		{ print }
	' "$dst/system/react-drm-config-gui.desktop" >"$launcher_tmp"
	desktop-file-validate "$launcher_tmp"
	install -d -o "$target_user" -g "$target_group" -m 0755 "$app_dir"
	install -o "$target_user" -g "$target_group" -m 0644 \
		"$launcher_tmp" "$launcher_file"
	rm -f "$launcher_tmp"
	run_as_target update-desktop-database "$app_dir"

	service_dir="$target_home/.config/systemd/user"
	service_file="$service_dir/react-drm.service"
	env_q=$(systemd_escape_path "$dst/.env")
	workdir_q=$(systemd_escape_path "$dst/linux-touchbar-control-center")
	start_q=$(systemd_escape_path "$dst/linux-touchbar-control-center/dist/index.js")
	detach_q=$(systemd_escape_path "$dst/system/react-drm-tb-detach")

	info "installing react-drm user service"
	install -d -o "$target_user" -g "$target_group" -m 0755 "$service_dir"
	temporary_file=$(mktemp --suffix=.service /tmp/react-drm-kait2en.XXXXXX)
	if ! awk -v envfile="$env_q" -v workdir="$workdir_q" -v start="$start_q" -v detach="$detach_q" '
		/^EnvironmentFile=/ { print "EnvironmentFile=-" envfile; next }
		/^WorkingDirectory=/ { print "WorkingDirectory=" workdir; next }
		/^ExecStart=/ { print "ExecStart=node " start; next }
		/^ExecStopPost=/ { print "ExecStopPost=-" detach; next }
		{ print }
	' "$dst/system/react-drm.service" >"$temporary_file"; then
		rm -f "$temporary_file"
		fail "unable to generate react-drm user service"
	fi
	install -o "$target_user" -g "$target_group" -m 0644 "$temporary_file" "$service_file"
	rm -f "$temporary_file"

	run_as_target systemctl --user daemon-reload
	if run_as_target systemctl --user is-active --quiet react-drm.service; then
		run_as_target systemctl --user stop react-drm.service
	fi
	if [[ ${#missing_groups[@]} -gt 0 ]]; then
		run_as_target systemctl --user enable react-drm.service
		info "react-drm will start after $target_user logs out and back in"
	else
		run_as_target systemctl --user enable --now react-drm.service
		sleep 2
		run_as_target systemctl --user is-active --quiet react-drm.service ||
			fail "react-drm failed to remain active; inspect it with 'journalctl --user -u react-drm.service -b'"
	fi
}

if [[ "$install_mode" == all ]]; then
	install -d -o root -g root -m 0755 /usr/local/share/kait2en
	install -o root -g root -m 0644 \
		"$REPO_ROOT/assets/kait2en-app-logo.png" \
		/usr/local/share/kait2en/kait2en-wordmark.png
	install_kait2en_fonts
	remove_obsolete_apps
	install_rust_app "$REPO_ROOT/apps/t2-fan-control" "t2-fan-control"
	install_rust_app "$REPO_ROOT/apps/t2-smc-control" "t2-smc-control"
	install_rust_app "$REPO_ROOT/apps/t2-power-explorer" "t2-power-explorer"
	install_gpu_control
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
fi
install_react_drm

if [[ "$install_mode" == react-drm ]]; then
	info "react-drm installed"
else
	info "apps installed"
fi
