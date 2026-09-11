#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dnf

KVER="$(kernel_release)"

if [[ "$KVER" == *.fc*.x86_64 ]]; then
	development_package=("kernel-devel-$KVER")
else
	build_link="/lib/modules/$KVER/build"
	[[ -f "$build_link/Makefile" ]] ||
		fail "custom kernel build tree is unavailable at $build_link"
	build_tree=$(readlink -f "$build_link")
	info "using custom kernel build tree $build_tree"
	development_package=()
fi

info "installing Fedora build and runtime dependencies for $KVER"
packages=(
	plymouth-plugin-script \
	acpica-tools \
	alsa-ucm \
	dkms \
	gcc \
	gcc-c++ \
	make \
	python3 \
	pkgconf-pkg-config \
	"${development_package[@]}" \
	kernel-headers \
	elfutils-libelf-devel \
	dracut \
	grubby \
	polkit \
	cargo \
	rust \
	gtk4-devel \
	libadwaita-devel \
	glib2-devel \
	systemd-devel \
	libdrm-devel \
	cairo-devel \
	librsvg2-devel \
	libplist-devel \
	fprintd \
	fprintd-pam \
	libfprint \
	checkpolicy \
	policycoreutils-python-utils \
	nodejs \
	npm \
	brightnessctl \
	cava
)

failed_packages=()
for package in "${packages[@]}"; do
	if ! dnf install -y "$package"; then
		warn "failed to install dependency $package; continuing"
		failed_packages+=("$package")
	fi
done

if (( ${#failed_packages[@]} > 0 )); then
	warn "dependency installation completed with errors in: ${failed_packages[*]}"
fi

info "dependencies installed"
