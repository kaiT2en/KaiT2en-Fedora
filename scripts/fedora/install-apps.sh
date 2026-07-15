#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command \
	awk cargo chown cut dnf env getent grep id install make mktemp npm rm rpm \
	sleep sudo systemctl tar tr udevadm usermod

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

remove_obsolete_apps() {
	info "removing obsolete t2-gpu-switch installation"
	rm -f \
		/usr/local/bin/t2-gpu-switch \
		/usr/local/libexec/t2-gpu-switch-helper \
		/usr/local/share/applications/org.t2gpuswitch.gtk.desktop
}

install_rust_app() {
	local path=$1 name=$2 target_user
	info "building and installing $name"

	target_user="${SUDO_USER:-}"
	[[ -n "$target_user" && "$target_user" != root ]] ||
		fail "$name must be built for the user who invoked sudo"

	sudo -H -u "$target_user" make -C "$path" clean
	sudo -H -u "$target_user" make -C "$path" build
	make -C "$path" install
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

install_react_drm() {
	local target_user target_home target_uid target_group src dst
	local installed_node_packages=() package daemon unit group groups
	local missing_groups=()
	local service_dir service_file temporary_file workdir_q start_q detach_q
	if ! has_t2_touchbar_model; then
		return
	fi

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
	for package in package.json package-lock.json system/99-react-drm.rules system/react-drm.service system/react-drm-tb-detach; do
		[[ -r "$src/$package" ]] || fail "react-drm deployment file is missing: $package"
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
		systemctl disable --now "$unit" >/dev/null 2>&1 || true
		run_as_target systemctl --user disable --now "$unit" >/dev/null 2>&1 || true
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
	install -o root -g root -m 0644 "$src/system/99-react-drm.rules" /etc/udev/rules.d/99-react-drm.rules
	udevadm control --reload
	udevadm trigger --action=add --subsystem-match=usb --subsystem-match=backlight
	udevadm trigger --action=add --subsystem-match=misc --sysname-match=uinput

	info "copying react-drm source to $dst"
	rm -rf "$dst"
	install -d -o "$target_user" -g "$target_group" -m 0755 "$dst"
	tar -C "$src" \
		--exclude='.git' \
		--exclude='node_modules' \
		--exclude='dist' \
		--exclude='linux-touchbar-control-center/dist' \
		-cf - . | tar -C "$dst" -xf -
	chown -R "$target_user:$target_group" "$dst"

	info "building react-drm"
	run_as_target npm --prefix "$dst" ci
	run_as_target npm --prefix "$dst/linux-touchbar-control-center" run build

	service_dir="$target_home/.config/systemd/user"
	service_file="$service_dir/react-drm.service"
	workdir_q=$(systemd_escape_path "$dst/linux-touchbar-control-center")
	start_q=$(systemd_escape_path "$dst/linux-touchbar-control-center/dist/index.js")
	detach_q=$(systemd_escape_path "$dst/system/react-drm-tb-detach")

	info "installing react-drm user service"
	install -d -o "$target_user" -g "$target_group" -m 0755 "$service_dir"
	temporary_file=$(mktemp --suffix=.service /tmp/react-drm-kait2en.XXXXXX)
	if ! awk -v workdir="$workdir_q" -v start="$start_q" -v detach="$detach_q" '
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
	run_as_target systemctl --user enable --now react-drm.service
	sleep 2
	run_as_target systemctl --user is-active --quiet react-drm.service ||
		fail "react-drm failed to remain active; inspect it with 'journalctl --user -u react-drm.service -b'"
}

remove_obsolete_apps
install_rust_app "$REPO_ROOT/apps/t2-fan-control" "t2-fan-control"
install_rust_app "$REPO_ROOT/apps/t2-smc-control" "t2-smc-control"
install_react_drm

info "apps installed"
