![KAIT2EN logo](../assets/kaiT2en-logo-tr.png){ .kait2en-page-logo }

# Introduction

KAIT2EN adds Intel T2 Mac support to stock Fedora without replacing Fedora's
kernel. Hardware support is provided by DKMS modules, system configuration and
small desktop tools maintained in this repository. Fedora kernel updates remain
normal Fedora updates; DKMS rebuilds the out-of-tree modules for each new
kernel.

KAIT2EN supports Fedora only. The project deliberately uses an otherwise stock
Fedora installation so hardware problems can be reproduced, fixed and moved
toward upstream Linux without distribution-specific kernel patches obscuring
the result.

## Choose an installation path

### Automatic installation

The [automatic installer](installation/automatic/index.md) is the recommended
path for a new Fedora installation. Run it from macOS with an empty USB drive.
It prepares an official Fedora installer with temporary KAIT2EN boot support,
copies the matching Apple firmware and continues the setup after Fedora has
been installed.

Internal keyboard and trackpad work in the automatic installer. The Fedora
installation payload and kernel remain stock; KAIT2EN is installed afterward
as a separate driver, configuration and tooling layer.

### Manual installation

Use the manual path when you need to inspect or control every installation
step. Start by [copying the Broadcom firmware from
macOS](installation/manual/00-get-broadcom-firmware.md), then prepare a standard
Fedora installer and install KAIT2EN from this repository.

A plain Fedora installer does not contain the required T2 input drivers. On a
MacBook, the manual path therefore requires a wired USB keyboard and mouse, or
wireless devices with their own USB receiver. Bluetooth input devices will not
work at that stage. Wi-Fi also becomes available only after the Apple firmware
has been installed.

Keep a separate copy of this repository and the firmware files. Fedora Media
Writer and `dd` overwrite the installer drive, so using a second USB drive is
the least error-prone manual setup.

### Existing T2 Linux Fedora installation

KAIT2EN can also replace the patched T2 Linux stack on an existing Fedora
installation. Install the KAIT2EN modules and applications first, then follow
the [migration guide](migration/revert-t2linux-fedora.md) to remove conflicting
T2 Linux packages and return to Fedora's stock kernel.

Do not keep both driver stacks active. They provide competing modules and
configuration for the same hardware.

## Before you start

KAIT2EN is intended for Intel Macs with an Apple T2 security chip. Back up
important data before changing partitions or boot settings.

**Keep macOS installed.** It is the clean source for model-specific Apple
firmware, can recover T2 and bridgeOS hardware states, and is the only place
where bridgeOS panic logs are available. If macOS must be removed completely,
this installation path is not supported.

Create a real partition for Fedora from macOS Disk Utility. Do not delete the
EFI partition or the macOS installation. Apple Secure Boot must be disabled and
booting from external media must be allowed before a standard Fedora installer
can start.

Firmware copied from macOS is never distributed by KAIT2EN. The installer and
manual guide only copy firmware already present on your own Mac and install it
locally.

## After installation

To update KAIT2EN, pull the latest repository changes and run the main installer
from the repository root:

```bash
sudo bash ./scripts/fedora/install.sh
```

See [Updating KAIT2EN](postinstall/updating.md) for details.

Some T2 models still require specific GPU configuration or have unresolved
hardware limitations. Review [Configure GPUs](postinstall/configuring-gpus.md)
before changing graphics modes or testing suspend on a dual-GPU Mac.

For installation problems, open an issue on
[GitHub](https://github.com/kaiT2en/KaiT2en-Fedora), join the
[Discord community](https://discord.gg/AGfjRk4ydj), or use the
[KAIT2EN Matrix space](https://matrix.to/#/%23kait2en:matrix.org).
