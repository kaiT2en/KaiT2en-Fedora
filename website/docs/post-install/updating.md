# Updating

KAIT2EN carries fixes until they are upstream. Merged fixes leave the repo.
Until then, updates bring new module, app and installer versions.

## Updating KAIT2EN

Open a terminal and run:

```bash
kait2en-install
```

This updates the KAIT2EN Git checkout and runs the project installer. Reboot
after it completes successfully.

If `kait2en-install` fails, clone the repo and run the installer from there:

```bash
git clone https://github.com/kaiT2en/KaiT2en-Fedora.git ~/KaiT2en-Fedora
cd ~/KaiT2en-Fedora/scripts/fedora
sudo ./install.sh
```

[KaiT2en Update](applications/kait2en-update.md) is an experimental app that
installs only what changed. [Multicall commands](multicall-commands.md) update
only the modules or only the apps.

## Updating Fedora

Update Fedora as usual. DKMS rebuilds the KAIT2EN modules for each new kernel.

On Macs with the patched AMDGPU module (MacBookPro15,1, 16,1, 16,4, iMac20,1
and iMac20,2), a kernel-install hook also rebuilds AMDGPU for the new kernel.
It downloads the kernel source RPM, so it needs network access and adds a few
minutes to the update. dnf shows no output meanwhile. Reboot only after dnf has
finished. The build log is `/var/log/kait2en-gpu-runtime-pm.log`.

If that build failed, the new kernel boots with Fedora's stock AMDGPU. Rebuild
it and reboot:

```bash
sudo /usr/local/libexec/kait2en/gpu-runtime-pm/install-gpu-runtime-pm.sh install <kernel>
sudo reboot
```

`<kernel>` is the new kernel version, for example `uname -r` after booting it.
See [Configuring GPUs](configuring-gpus.md) for details.

Kernel regressions are reported on [Discord](https://discord.gg/AGfjRk4ydj) and
[Matrix](https://matrix.to/#/%23kait2en:matrix.org).

## Repairing a broken kernel update

An interrupted DKMS build, for example by a reboot during the update, leaves
incomplete modules for the new kernel. Boot an older kernel from GRUB and
rebuild the new one:

```bash
# replace 7.1.3-201.fc44.x86_64 with the broken kernel version
sudo dracut --force /boot/initramfs-7.1.3-201.fc44.x86_64.img 7.1.3-201.fc44.x86_64
sudo kernel-install add 7.1.3-201.fc44.x86_64 /lib/modules/7.1.3-201.fc44.x86_64/vmlinuz
sudo grubby --set-default /boot/vmlinuz-7.1.3-201.fc44.x86_64
```

A faulty update can disable the internal keyboard and trackpad or Wi-Fi. Keep an
external keyboard and mouse at hand when updating, for example while traveling.

### When you can't boot at all

If the system no longer boots or the input drivers are missing, boot the
KAIT2EN USB drive into the live desktop and run:

```bash
sudo kait2en-rescue
```

The rescue tool finds and mounts your Fedora installation, chroots into it,
and lets you rebuild the initramfs, verify the T2 input drivers or choose the
default boot entry.
