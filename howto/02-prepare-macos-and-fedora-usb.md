# Prepare macOS and the Fedora installer

[Automatic installation](automatic-installation.md)

Manual installation:

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md) (you are here)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md)
8. [How to update](07-updating.md)

Previous: [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md) | Next: [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)

*When you already have T2 Linux Fedora installed and you are using our revert
script, you can jump to [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md).*

## Prerequisites

Internal input devices won't work during installation of Fedora,
because the stock kernel does not contain the needed drivers.
We will add them in a later step.

In effect, an external keyboard and mouse is mandatory for this step.
Bluetooth keyboards/mice will also not work. Logitech wireless devices
or similar, that come with their own receiver will work, as well as
regular wired USB keyboards/mice.

## Do this before booting the Fedora installer

KaiT2en assumes that macOS stays installed. If you want to erase macOS
completely, this guide is not for you. Use the regular T2 Linux documentation
instead.

macOS is required because it gives you a clean source for Apple firmware, can
settle the T2 after firmware or bridgeOS trouble, and is the only place where
you can read bridgeOS panic logs. In practice, booting macOS can recover hardware
states that do not come back with a PRAM or SMC reset.

Use two USB drives if possible:

- one USB drive for the KaiT2en repository
- one USB drive for the Fedora installer

Fedora Media Writer and `dd` overwrite the installer USB drive. Do not write the
Fedora image to the USB drive that contains the KaiT2en repository unless you
have another copy of the repository somewhere else.

## Create space for Linux in macOS

Open Disk Utility in macOS.

Select the internal macOS disk or container and choose `Partition`. Add a real `exFAT`
partition for Linux. Do not add an APFS volume.

The temporary format does not matter much, because Fedora will reformat
this partition during installation. `exFAT` is a reasonable choice because
it is easy to recognize later.

Choose the size carefully. Resizing after Linux is installed is possible, but it
is not a small maintenance task.

If disk space is tight, shrink macOS to about `50 GB` and give the remaining
space to Linux. Keep enough macOS space for updates, firmware work and recovery
tasks.

Do not delete the EFI partition. Do not delete macOS.

## Disable Secure Boot in macOS Startup Security Utility

Shut down the Mac.

Turn it on and hold `Command-R` until macOS Recovery starts. Select your macOS
user and enter the password.

Open `Utilities` and then `Startup Security Utility`.

Set:

```text
Secure Boot: No Security
Allowed Boot Media: Allow booting from external or removable media
```

This is required because Apple Secure Boot does not boot standard Fedora while
it is enabled.

## Create the Fedora installer USB in macOS

Download the standard Fedora `x86_64` Workstation image from:

```text
https://fedoraproject.org/workstation/download/
```

Use the standard Fedora image, not a T2 Linux Fedora image.

The easiest method on macOS is Fedora Media Writer. It is available from the
same Fedora Workstation download page and can download Fedora Workstation for
you.

```text
https://fedoraproject.org/workstation/download/
```

Select Fedora Workstation and write it to the installer USB drive.

If you prefer the terminal, use `dd`. This destroys all data on the selected USB
drive.

Insert the Fedora installer USB drive and find its disk number:

```bash
diskutil list
```

Look for the external physical disk, for example `/dev/disk2`. Replace `X` below
with the correct number:

```bash
sudo diskutil unmountDisk /dev/diskX
sudo dd if=/path/to/Fedora-Workstation-Live.iso of=/dev/rdiskX bs=1m status=progress
sync
```

Be exact. Writing to the wrong disk can destroy macOS or other data.

## Boot the Fedora installer

Connect the Fedora installer USB drive.

Also keep the KaiT2en USB drive connected if you already copied this repository
and the firmware files to it.

Shut down or reboot the Mac. Hold `Option` during startup and select the orange
`EFI Boot` entry for the Fedora installer.

Use an external keyboard and mouse for the installation. The internal keyboard
and trackpad are not expected to work yet on a plain Fedora installer.
T2 Mac specific devices will work after installing the modules in step 04.

During Fedora installation, choose manual partitioning. Use the Linux partition
you created in macOS. Do not use automatic partitioning. Do not erase the whole
disk.
The rest of the installation process is standard Fedora. Refer to:

```text
https://docs.fedoraproject.org/
```

After successful installation, continue with
[Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md).
