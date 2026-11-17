#!/usr/bin/env bash
#
# Reverts a T2 Linux Fedora installation to vanilla Fedora + KAIT2EN.
# Removes the sharpenedblade/t2linux COPR, the t2linux support packages
# (t2linux-config, t2linux-audio, t2fanrd, tiny-dfr, ...) and their config
# leftovers, resets stored WirePlumber defaults, distro-syncs the kernel back
# to the Fedora repositories and rebuilds the initramfs for non-.t2 kernels.
# Run it AFTER the KAIT2EN installer has completed. KAIT2EN files are left
# untouched. Reboot afterwards.
#
# Restored from howto/migration/revert-t2linux-fedora.md (removed in 10ac212).
#
# Author: André Eikmeyer (dev@deqrocks)
# Date: 2026-07-13
#
# This script is provided without warranty. Use it at your own risk.
# The author and project contributors are not responsible for data loss,
# hardware damage, system failure, or any other consequences of its use.

set -euo pipefail
shopt -s nullglob

if [[ $EUID -ne 0 ]]; then
    echo "Please run as root (sudo $0)" >&2
    exit 1
fi

info() {
    printf '==> %s\n' "$*"
}

warn() {
    printf 'WARNING: %s\n' "$*" >&2
}

remove_if_exists() {
    local path
    for path in "$@"; do
        if [[ -e "$path" ]]; then
            info "removing $path"
            rm -rf -- "$path"
        elif [[ -L "$path" ]]; then
            info "removing $path"
            rm -rf -- "$path"
        fi
    done
}

disable_unit_if_present() {
    local unit=$1

    if [[ -e "/usr/lib/systemd/system/$unit" ]]; then
        info "disabling $unit"
        systemctl disable --now "$unit"
    elif [[ -e "/etc/systemd/system/$unit" ]]; then
        info "disabling $unit"
        systemctl disable --now "$unit"
    fi
}

disable_unit_if_present t2fanrd.service
disable_unit_if_present tiny-dfr.service
disable_unit_if_present get-apple-firmware.service

info "removing T2 Linux Fedora packages"
packages=(
    t2linux-release
    t2linux-repos
    copr-sharpenedblade-t2linux-release
    t2linux-repo
    t2linux-config
    t2linux-audio
    t2linux-scripts
    t2fanrd
    tiny-dfr
    tinydfr
    mac-touchbar-plus
)
installed_packages=()
for package in "${packages[@]}"; do
    if rpm -q "$package" >/dev/null 2>&1; then
        installed_packages+=("$package")
    fi
done
if [[ "${#installed_packages[@]}" -gt 0 ]]; then
    dnf remove -y "${installed_packages[@]}"
else
    info "no T2 Linux Fedora packages from the known package list are installed"
fi

info "removing T2 Linux COPR repo files"
remove_if_exists \
    /etc/yum.repos.d/copr-sharpenedblade-t2linux.repo \
    /etc/yum.repos.d/_copr:copr.fedorainfracloud.org:sharpenedblade:t2linux.repo

info "removing T2 Linux system configuration leftovers"
remove_if_exists \
    /usr/lib/modules-load.d/t2linux-modules.conf \
    /usr/lib/dracut/dracut.conf.d/t2linux-modules-install.conf \
    /usr/lib/udev/rules.d/90-network-t2-ncm.rules \
    /usr/lib/NetworkManager/conf.d/90-network-t2-ncm.conf \
    /usr/lib/systemd/system-preset/91-t2linux.preset \
    /usr/lib/udev/rules.d/91-audio-t2.rules \
    /usr/lib/udev/rules.d/99-touchbar-seat.rules \
    /usr/lib/udev/rules.d/99-touchbar-tiny-dfr.rules \
    /usr/share/alsa-card-profile/mixer/paths/t2-builtin-mic.conf \
    /usr/share/alsa-card-profile/mixer/paths/t2-headphones.conf \
    /usr/share/alsa-card-profile/mixer/paths/t2-headset-mic.conf \
    /usr/share/alsa-card-profile/mixer/paths/t2-speakers.conf \
    /usr/share/alsa-card-profile/mixer/profile-sets/apple-t2x2.conf \
    /usr/share/alsa-card-profile/mixer/profile-sets/apple-t2x4.conf \
    /usr/share/alsa-card-profile/mixer/profile-sets/apple-t2x6.conf \
    /usr/share/pulseaudio/alsa-mixer/paths/t2-builtin-mic.conf \
    /usr/share/pulseaudio/alsa-mixer/paths/t2-headphones.conf \
    /usr/share/pulseaudio/alsa-mixer/paths/t2-headset-mic.conf \
    /usr/share/pulseaudio/alsa-mixer/paths/t2-speakers.conf \
    /usr/share/pulseaudio/alsa-mixer/profile-sets/apple-t2x2.conf \
    /usr/share/pulseaudio/alsa-mixer/profile-sets/apple-t2x4.conf \
    /usr/share/pulseaudio/alsa-mixer/profile-sets/apple-t2x6.conf

systemctl daemon-reload
udevadm control --reload

info "removing stored WirePlumber defaults for Apple T2 audio"
if [[ -n "${SUDO_USER:-}" && "${SUDO_USER:-}" != root ]]; then
    target_home=""
    if passwd_entry="$(getent passwd "$SUDO_USER")"; then
        IFS=: read -r _ _ _ _ _ target_home _ <<< "$passwd_entry"
    fi
else
    target_home="${HOME:-/root}"
fi
if [[ -d "$target_home/.local/state/wireplumber" ]]; then
    cp -a "$target_home/.local/state/wireplumber" \
        "$target_home/.local/state/wireplumber.pre-t2linux-revert.$(date +%Y%m%d%H%M%S)"
    rm -f "$target_home/.local/state/wireplumber/default-profile" \
          "$target_home/.local/state/wireplumber/default-nodes"
fi

info "switching kernel packages back to enabled Fedora repositories"
if ! dnf distro-sync -y \
    kernel \
    kernel-core \
    kernel-modules \
    kernel-modules-core \
    kernel-modules-extra \
    kernel-devel \
    kernel-headers; then
    warn "Kernel package sync failed. Install a Fedora kernel manually before rebooting."
fi

info "checking installed Fedora kernels"
found_fedora_kernel=0
for kernel_release in $(rpm -q kernel-core --queryformat '%{VERSION}-%{RELEASE}.%{ARCH}\n'); do
    case "$kernel_release" in
        *.t2*)
            continue
            ;;
    esac

    found_fedora_kernel=1
    info "rebuilding initramfs for $kernel_release"
    dracut --force "/boot/initramfs-$kernel_release.img" "$kernel_release"
done

if [[ "$found_fedora_kernel" != 1 ]]; then
    warn "No non-.t2 Fedora kernel-core package is installed."
    warn "Install a Fedora kernel manually before rebooting."
    exit 1
fi

info "installed kernels:"
rpm -qa 'kernel-core*'

info "done. Reboot into Fedora."
