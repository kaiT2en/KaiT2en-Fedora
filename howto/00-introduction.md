<p align="center">
  <img src="../assets/kaiT2en-logo-tr.png" alt="KaiT2en logo" width="220">
</p>

# Introduction

1. [Introduction](00-introduction.md) (you are here)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)

[Back to README](../README.md) | Next: [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)

KaiT2en [ˈkaɪ̯zɛn] refers to the Japanese philosophy of "kaizen". Which
means constant small improvements.

Kait2en is using DKMS modules to add T2 Mac driver support to stock Fedora.

It also ships with the latest workarounds to make suspend working and includes
T2 specific apps and tools. 

The DKMS modules in this repository build against the currently installed
kernel. This makes it possible for developers to quickly test patches without the need
of recompiling the kernel. Also users can profit quickly from the latest efforts.
Modules and Kernels are updated separately. 
So you will always get the latest
kernel from upstream Fedora and decide for yourself if you want to update
the modules from this repo.

KaiT2en can also be installed on top of existing T2linux.org kernels.
It blacklists already upstreamed drivers and replaces them with its own.
*Note there will be still remnants of T2linux stuff when doing so, what is
not helpful when dealing with issues.* That's why we recommend installing 
KaiT2en on top of stock Fedora.

KaiT2en will not work on other distros than Fedora. This was a deliberate choice.
In the first place we want a unified clean platform for debugging. We do not
support ports to other distros.

The repository is meant to be used as an offline USB kit. Copy it to a USB
drive, keep that drive connected, and run all commands from the repository root
unless a guide says otherwise.

The setup is intentionally explicit. You will use the terminal, inspect logs and
know which file was installed where. 

**On clean Fedora install internal keyboard and trackpad won't work on MacBooks.
Also WiFi will not. USB ethernet will work though. 
This repository is meant to be copied to a USB drive before installation. Keep
that drive connected while working through these guides. Whenever a guide asks you
to run a script or paste a command block, open a terminal in the root folder of
this repository first.** The guides and scripts use relative paths, so files are 
expected below that folder.

**macOS must stay installed.** It is the clean source for Apple firmware, it can
recover T2/bridgeOS hardware states, and it is the only place where bridgeOS
panic logs are available. If you want to erase macOS completely, KaiT2en is the
wrong path. We are focussed on fixing and debugging things. Don't ask for support
when using custom installations.

Each of our guides does one job. Read it, run the commands, check the result,
then move to the next guide. The point is to make the setup understandable
and repairable.

Firmware files copied from macOS are never distributed by KaiT2en. The guides
only show how to copy the correct firmware files from your own Mac and install
them locally.

So for now best luck for the installation. If you happen to run into issues or inconsistencies,
please open a PR or file an issue.

Next: [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
