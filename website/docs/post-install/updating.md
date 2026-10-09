# How to update

KAIT2EN is meant to disappear. It's part of the concept. It serves the purpose
of upstreaming code fixes. Every module and fix that gets upstreamed will disappear from
the repo. Until we are left with a few T2 specific apps and other things that can't be upstreamed.
Maybe it will survive as a collection of helpful apps for T2 Macs.

As long as this isn't the case, we need to update the Fedora kernel and our modules and apps.
The question how to do that has kept us a bit busy. We could go COPR and offer .rpm.
But the project is yet too small to fund itself.

## Updating KAIT2EN

Open a terminal and run:

```bash
kait2en-install
```

This updates the KAIT2EN Git checkout and runs the regular project installer.
Review its output and reboot after it completes successfully.

If that fails for whatever reason, you can always clone our repo directly to
a folder of your choice and run the main install script directly.
So for example, open a terminal and type `cd`. This will take you to your
username's home directory. Then type 

```bash
git clone https://github.com/kaiT2en/KaiT2en-Fedora.git
```

This will create a folder `KaiT2en-Fedora` in your home dir which contains our latest code.
Then simply type:

```bash
cd KaiT2en-Fedora/scripts/fedora
sudo ./install.sh
```

This will run the installer and update your KAIT2EN installation just like
`kait2en-install` would do.

### Hybrid graphics and kernel updates

On a MacBookPro15,1, 16,1 or 16,4, the patched graphics driver is rebuilt
during every kernel update. dnf does not show its output, so the update simply
takes a few minutes longer. Reboot only after dnf has finished. The build log
is written to `/var/log/kait2en-gpu-runtime-pm.log`.

If the build failed, rebuild the driver for the new kernel and reboot:

```bash
cd /usr/local/src/KaiT2en-Fedora/scripts/fedora
sudo ./install-gpu-runtime-pm.sh install <kernel>
```

`<kernel>` is the version of the new kernel, for example the output of
`uname -r` after booting it. `kait2en-install` rebuilds the driver for the
running kernel as part of its normal run.

## Updating Fedora

You just update Fedora like everyone else. DKMS will notice and recompile our modules against the
latest kernel. On Macs that use the patched AMDGPU module (MacBookPro15,1, 16,1, 16,4 and the 5K
iMacs), a KAIT2EN hook also rebuilds it for the new kernel. This needs network access and makes
kernel updates take a few minutes longer. dnf stays quiet meanwhile; the build log is in
`/var/log/kait2en-gpu-runtime-pm.log`. See [How to configure GPUs](configuring-gpus.md) if it fails. It's always worth visiting the [KAIT2EN community on Discord](https://discord.gg/AGfjRk4ydj) or [Matrix](https://matrix.to/#/%23kait2en:matrix.org) to make sure you won't run into issues like kernel regressions.

## So you messed up?

You can mess up DKMS when upgrading the kernel. For example when interrupting DKMS while the new Kernel is booting.
Then you are left with half broken KAIT2EN modules. To repair this, just boot into an older kernel and clean up the new.

```
#replace the kernel version 7.1.3-201.fc44.x86_64 with your own
sudo dracut --force /boot/initramfs-7.1.3-201.fc44.x86_64.img 7.1.3-201.fc44.x86_64 
sudo kernel-install add 7.1.3-201.fc44.x86_64 /lib/modules/7.1.3-201.fc44.x86_64/vmlinuz
sudo grubby --set-default /boot/vmlinuz-7.1.3-201.fc44.x86_64
```

### When you can't boot at all

If you can no longer boot your system or the input drivers are not loaded, use
the KAIT2EN USB stick. Boot into a live desktop and run:

```bash
sudo kait2en-rescue
```

The rescue tool finds and mounts your Fedora installation, chroots you into it,
and lets you rebuild the initramfs, verify the T2 input drivers, or choose the
default boot entry.

## Murphy's law: We **WILL** mess up!

We are humans. We will mess up at some point. We recommend not to update when
you don't have an external keyboard/mouse around. Like when you are travelling.
But you should always be able to use GRUB to boot into an older kernel anyways.
But be warned that when we mess up, you could loose VHCI devices or WiFi.
And remember we are not paid. We will waste your time and we don't accept complaints.
