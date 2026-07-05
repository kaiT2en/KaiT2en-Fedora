#!/usr/bin/env bash

source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/lib.sh"

require_root
require_repo_root
require_fedora
require_command dnf

KVER="$(kernel_release)"

info "installing Fedora build and runtime dependencies for $KVER"
dnf install -y \
	alsa-ucm \
	dkms \
	gcc \
	gcc-c++ \
	make \
	python3 \
	pkgconf-pkg-config \
	kernel-devel-"$KVER" \
	kernel-headers \
	elfutils-libelf-devel \
	dracut \
	grubby \
	polkit \
	cargo \
	rust \
	gtk4-devel \
	libadwaita-devel \
	systemd-devel \
	libdrm-devel \
	cairo-devel \
	librsvg2-devel \
	nodejs \
	npm \
	brightnessctl \
	cava

info "dependencies installed"
