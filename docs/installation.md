<p align="center">
  <img src="../assets/kaiT2en-logo-tr.png" alt="KAIT2EN logo" width="220">
</p>

# Installation

The installer prepares one Fedora USB drive on macOS. Internal
keyboard, trackpad, and Wi-Fi work in the live system and during Fedora
installation. The matching Apple Wi-Fi firmware and the guided KAIT2EN setup are
carried into the installed system.

The installer currently supports Fedora Workstation, Fedora KDE Desktop and
Fedora COSMIC Spin.

## Before you start

You need:

- a T2 Mac with macOS still installed
- an empty USB drive
- an internet connection in macOS

Back up important data before changing partitions or boot settings.

Keep macOS installed. It is the clean source for Apple firmware and can recover
T2/bridgeOS hardware states.

Open Disk Utility in macOS and create a real `exFAT` partition for Fedora. Do
not add an APFS volume, delete the EFI partition or delete macOS. Fedora will
reformat the new partition during installation.

Apple Secure Boot must be disabled and booting from external media must be
allowed. The installer checks the current setting and shows the short Recovery
steps when it needs to be changed.

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
you can connect to a network before or instead of installing Fedora. If no
wireless network appears, open a terminal and run this command:

```bash
sudo /run/kait2en/kait2en-live-wifi
```

Macs whose Bluetooth controller sits on PCIe (BCM4377) also get their Apple
Bluetooth firmware in the live system, so a Bluetooth keyboard or mouse can be
paired before installing. Retry that with:

```bash
sudo /run/kait2en/kait2en-live-bluetooth
```

Every other T2 Mac drives Bluetooth over UART and needs no firmware file, so
this command reports that there is nothing to do.

If that does not help, collect diagnostics for a bug report:

```bash
sudo /run/kait2en/kait2en-live-diagnostics --rerun
```

This retries the Wi-Fi and Bluetooth setup, records what happened, and writes
one archive.
It lands on a second USB drive when one is mounted, otherwise in `/tmp`; the
path is printed at the end. The archive contains host names, MAC addresses, and
the names of nearby wireless networks, so look at it before passing it on.

Install Fedora normally. Use custom partitioning and select the Linux partition
you created in macOS. Do not erase the whole disk or macOS. When reinstalling,
format an existing Linux `/boot` partition so old kernels do not fill it.

After installation finishes, remove the USB drive and boot the installed Fedora
system.

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
