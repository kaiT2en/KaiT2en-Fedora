#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
set -Eeuo pipefail
APP_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$APP_DIR/../../scripts/fedora/lib.sh"
require_root
install_kait2en_fonts
require_fedora
require_command dnf install find rm

required_files=(
	"$APP_DIR/t2-kernel-builder.py"
	"$APP_DIR/t2-kernel-builder-cleanup"
	"$APP_DIR/org.t2kernelbuilder.gtk.policy"
	"$APP_DIR/org.t2kernelbuilder.gtk.desktop"
	"$APP_DIR/org.t2kernelbuilder.gtk.svg"
	"$APP_DIR/engine/build.sh"
)
for required_file in "${required_files[@]}"; do
	[[ -f "$required_file" ]] || fail "t2-kernel-builder bundle is incomplete: $required_file"
done
[[ -d "$APP_DIR/engine/configs" ]] ||
	fail "t2-kernel-builder bundle is incomplete: $APP_DIR/engine/configs"
find "$APP_DIR/engine/configs" -maxdepth 1 -type f -name '*.config' -print -quit |
	grep -q . || fail "t2-kernel-builder bundle contains no kernel configs"

dnf install -y bc binutils bison cpio curl dkms dracut dwarves \
	elfutils-libelf-devel flex gcc git-core gtk4 kmod libadwaita make \
	openssl-devel patch perl-core python3-gobject python3-pip polkit rpm-build \
	rsync tar xz
if ! python3 -c 'import importlib.metadata; raise SystemExit(importlib.metadata.version("kconfiglib") != "14.1.1a4")' \
		2>/dev/null; then
	PIP_NO_INPUT=1 python3 -m pip install --root-user-action=ignore --prefix=/usr/local \
		'https://github.com/sysprog21/Kconfiglib/archive/578b2c924cc673459c2f858ba902a47ad429a567.tar.gz'
fi
install -d -m 0755 /usr/local/bin /usr/local/libexec/t2-kernel-builder/configs \
	/usr/local/share/applications \
	/usr/local/share/icons/hicolor/scalable/apps /usr/local/share/polkit-1/actions
rm -rf /usr/local/libexec/t2-kernel-builder/patches
install -m 0755 "$APP_DIR/t2-kernel-builder.py" /usr/local/bin/t2-kernel-builder
install -m 0755 "$APP_DIR/t2-kernel-builder-cleanup" /usr/local/libexec/t2-kernel-builder-cleanup
install -m 0644 "$APP_DIR/org.t2kernelbuilder.gtk.policy" /usr/local/share/polkit-1/actions/
install -m 0755 "$APP_DIR/engine/build.sh" \
	/usr/local/libexec/t2-kernel-builder/build.sh
find "$APP_DIR/engine/configs" -maxdepth 1 -type f \
	-name '*.config' -exec install -m 0644 '{}' /usr/local/libexec/t2-kernel-builder/configs/ \;
install -m 0644 "$APP_DIR/org.t2kernelbuilder.gtk.desktop" /usr/local/share/applications/
install -m 0644 "$APP_DIR/org.t2kernelbuilder.gtk.svg" /usr/local/share/icons/hicolor/scalable/apps/
gtk-update-icon-cache --force --ignore-theme-index /usr/local/share/icons/hicolor
update-desktop-database /usr/local/share/applications
info "t2-kernel-builder installed"
