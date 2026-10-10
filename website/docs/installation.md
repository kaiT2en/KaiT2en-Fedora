# Installation

The installation starts in macOS and continues in Fedora:

1. In macOS Recovery, disable Secure Boot and allow booting from external media.
2. In macOS, create a separate partition for Fedora.
3. Boot macOS and run the KAIT2EN installer.
4. Choose a supported Fedora edition and an empty USB drive.
5. The installer downloads and verifies the official Fedora image.
6. It collects the Apple Wi-Fi firmware from your Mac and, where required, the
   PCIe Bluetooth firmware.
7. It writes the unchanged Fedora image to the USB drive, then adds separate
   KAIT2EN boot files for keyboard, trackpad, firmware, and installer support.
8. Boot the USB drive and install Fedora. After the first login, the guided
   KAIT2EN setup installs the remaining drivers and system integration.
9. After another reboot, you can enjoy KAIT2EN on top of vanilla Fedora.

The USB drive is a live installer. It only carries the drivers needed to install KAIT2EN.

The installer currently supports Fedora Workstation, Fedora KDE Desktop and
Fedora COSMIC Spin. We strongly recommend installing Workstation (Gnome)
because that is what KAIT2EN devs use. We still rely on your feedback on
other DE's. But if you want the most flawless experience, Gnome is the way.

## Before you start

You need:

- a T2 Mac with macOS still installed
- an empty USB drive
- an internet connection in macOS

Back up important data before changing partitions or boot settings.

Keep macOS installed. It is the clean source for Apple firmware and can recover
T2/bridgeOS hardware states.

### Disable Apple Secure Boot

- Shut down the Mac.
- Turn it on and immediately hold `Command-R` until macOS Recovery starts.
- Select a macOS administrator account and enter its password when prompted.
- From the menu bar, open `Utilities` > `Startup Security Utility`.
- Select the macOS system disk if the utility asks for one.
- Set the following options:

   `Secure Boot: No Security`

   `Allowed Boot Media: Allow booting from external or removable media`

- Close Startup Security Utility and restart into macOS.

Apple Secure Boot cannot boot standard Fedora while it is enabled. The installer
checks the current setting and warns when it cannot confirm that Secure Boot is
disabled.

### Create space for Fedora

Open Disk Utility in macOS. Select the internal macOS disk or container, choose
`Partition`, and add a real `exFAT` partition for Fedora. Do not add an APFS
volume. Fedora will reformat this partition during installation; `exFAT` only
makes it easy to identify.

Choose the size carefully because resizing the partitions later is not a small
maintenance task. Keep at least `50 GB` for macOS so there is enough space for
updates, firmware work, and recovery tasks. Give the remaining space you want
to use for Linux to the new partition.

Do not delete the EFI partition or macOS.

## Create the Fedora USB drive

Boot macOS and connect the empty USB drive. Open Terminal and run:

```bash
curl -fsSL https://github.com/kaiT2en/KaiT2en-Fedora/releases/latest/download/install-kait2en-fedora.sh | bash
```

Choose the Fedora desktop and USB drive. The script downloads and verifies the
official Fedora image, finds the Apple Wi-Fi firmware used by this Mac and asks
for an exact confirmation before erasing the USB drive.

The official Fedora image itself is not modified. After writing the verified
vanilla image, the script adds KAIT2EN boot files only to the USB drive's EFI
partition. Separate initramfs overlays provide the temporary input drivers and
installer integration at boot; Fedora's live system and installation payload
remain unchanged.

Be exact when selecting the drive. All data on it will be destroyed.

## Install Fedora

Shut down or reboot the Mac. Hold `Option` during startup and select the orange
`EFI Boot` entry for the Fedora USB drive. The KAIT2EN Fedora entry starts
automatically.

Keyboard, trackpad, and Wi-Fi should work in the live system and installer. The
live system installs the Apple Wi-Fi firmware from the USB drive for itself, so
you can connect to a network before or instead of installing Fedora.

Macs whose Bluetooth controller sits on PCIe (BCM4377) also get their Apple
Bluetooth firmware in the live system, so a Bluetooth keyboard or mouse can be
paired before installing. Every other T2 Mac drives Bluetooth over UART and
needs no separate firmware file.

Install Fedora normally. Use custom partitioning and select the Linux partition
you created in macOS. Do not erase the whole disk or macOS. When reinstalling,
format an existing Linux `/boot` partition so old kernels do not fill it.

After installation finishes, remove the USB drive and boot the installed Fedora
system.

### If the live system does not start

The boot menu offers a few troubleshooting entries that are only needed when the
normal boot does not work. Try them in this order and stop at the first one that
works:

1. `Troubleshooting: KaiT2en with boot messages` shows the console instead of
   the logo. Photograph the last lines for a bug report.
2. `Troubleshooting: KaiT2en with the dedicated GPU disabled` keeps `amdgpu`
   out of the boot and leaves the Intel GPU fully accelerated.
3. `Troubleshooting: KaiT2en with basic graphics` adds `nomodeset`. Rendering
   is done in software and feels slow, but it can run the installation.
4. `Troubleshooting: KaiT2en with basic graphics and conservative PCIe` is the
   last resort and also covers a power off coming from PCIe or Thunderbolt.

All of them keep the KAIT2EN input drivers. If entry 2 or 3 was needed, make
the fix permanent afterwards as described in
[Configure GPUs](post-install/configuring-gpus.md).

### If hardware is missing in the live system

KAIT2EN supports the known T2 configurations, but Apple shipped several
controller and firmware variants. If keyboard, trackpad, Wi-Fi, or PCIe
Bluetooth support is missing, collect diagnostics before continuing with the
installation.

The individual helpers can be run manually to see the Wi-Fi or Bluetooth setup
result directly:

```bash
sudo /run/kait2en/kait2en-live-wifi
sudo /run/kait2en/kait2en-live-bluetooth
```

For a complete diagnostic archive, run:

```bash
sudo /run/kait2en/kait2en-live-diagnostics --rerun
```

This reruns the firmware setup while recording what happened. The archive lands
on a second USB drive when one is mounted, otherwise in `/tmp`; the path is
printed at the end. It contains host names, MAC addresses, and the names of
nearby wireless networks, so inspect it before attaching it to a bug report.

## Finish the KAIT2EN installation

Sign in to Fedora and connect to Wi-Fi. A terminal opens automatically and
starts the KAIT2EN installer in two phases. Do not close this window and follow
the prompts.

The first phase updates Fedora and prepares the new kernel. Reboot when asked.
After signing in again, the second phase opens automatically and runs the
regular KAIT2EN installer. Reboot once more after it completes successfully.

If the terminal does not appear, open one and run this command without `sudo`:

```bash
kait2en-install
```

The installer asks for administrator access when it is needed. It can also be
started again at any later time to update KAIT2EN.
