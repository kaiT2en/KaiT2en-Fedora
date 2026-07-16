<p align="center">
  <img src="../assets/kaiT2en-logo-tr.png" alt="KaiT2en logo" width="220">
</p>

# Automatic installation

[Back to README](../README.md) | [Manual installation](00-introduction.md)

The automatic installer prepares one Fedora USB drive on macOS. Internal
keyboard and trackpad work during Fedora installation. The matching Apple Wi-Fi
firmware and the guided KaiT2en setup are carried into the installed system.

The installer currently supports Fedora Workstation, Fedora KDE Desktop and
Fedora COSMIC Spin.

## Before you start

You need:

- a T2 Mac with macOS still installed
- an empty USB drive
- an internet connection in macOS

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
vanilla image, the script adds KaiT2en boot files only to the USB drive's EFI
partition. Separate initramfs overlays provide the temporary input drivers and
installer integration at boot; Fedora's live system and installation payload
remain unchanged.

Be exact when selecting the drive. All data on it will be destroyed.

## Install Fedora

Shut down or reboot the Mac. Hold `Option` during startup and select the orange
`EFI Boot` entry for the Fedora USB drive. The KaiT2en Fedora entry starts
automatically.

Keyboard and trackpad should work in the live system and installer. Wi-Fi is not
expected to work in the live system; it becomes available after Fedora is
installed.

Install Fedora normally. Use manual partitioning and select the Linux partition
you created in macOS. Do not erase the whole disk or macOS. When reinstalling,
format an existing Linux `/boot` partition so old kernels do not fill it.

After installation finishes, remove the USB drive and boot the installed Fedora
system.

## Finish the KaiT2en installation

Sign in to Fedora and connect to Wi-Fi. A terminal opens automatically and
starts the KaiT2en installer in two phases. Do not close this window and follow
the prompts.

The first phase updates Fedora and prepares the new kernel. Reboot when asked.
After signing in again, the second phase opens automatically and runs the
regular KaiT2en installer. Reboot once more after it completes successfully.

If the terminal does not appear, open one and run this command without `sudo`:

```bash
kait2en-install
```

The installer asks for administrator access when it is needed. It can also be
started again at any later time to update KaiT2en.

[Back to README](../README.md) | [Manual installation](00-introduction.md)
