<p align="center">
  <img src="../assets/kaiT2en-logo-tr.png" alt="KaiT2en logo" width="220">
</p>

# Introduction

1. [Introduction](00-introduction.md) (you are here)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md)
8. [How to update](07-updating.md)

[Back to README](../README.md) | Next: [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)

**This repository is meant to be copied to a USB drive before installation. Keep
that drive connected while working through these guides**. Whenever a guide asks you
to run a script or paste a command block, do that in a terminal in the **root folder of
this repository**. The guides and scripts use relative paths, so files are 
expected below the KaiT2en USB root folder.

KaiT2en can be installed in two ways:
- clean install on top of vanilla Fedora. 
- clean install on top of existing T2 Linux Fedora and reverting to vanilla Fedora + Kait2en
as described in [05-revert-t2linux-fedora.md](howto/05-revert-t2linux-fedora.md).

On vanilla Fedora install internal keyboard and trackpad won't work on MacBooks.
Also WiFi will not. USB ethernet will work though.

The DKMS modules in this repository build against the currently installed
kernel. And they will rebuild automatically on kernel updates.

The setup is intentionally explicit. You will use the terminal, run commands and
know which file was installed where. The point is to make the setup understandable
and repairable.

**macOS must stay installed.** It is the clean source for Apple firmware, it can
recover T2/bridgeOS hardware states, and it is the only place where bridgeOS
panic logs are available. If you want to erase macOS completely, KaiT2en is the
wrong path. We are focussed on fixing and debugging things. Don't ask for support
when using custom installations.

Firmware files copied from macOS are never distributed by KaiT2en. The guides
only show how to copy the correct firmware files from your own Mac and install
them locally.

So for now best luck for the installation. If you happen to run into issues or inconsistencies,
please file an issue on GitHub or join the [KaiT2en community on Discord](https://discord.gg/AGfjRk4ydj)
or the [KaiT2en Matrix space](https://matrix.to/#/%23kait2en:matrix.org).

Next: [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
