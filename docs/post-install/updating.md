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

## Updating Fedora

You just update Fedora like everyone else. DKMS will notice and recompile our modules against the
latest kernel. It's always worth visiting the [KAIT2EN community on Discord](https://discord.gg/AGfjRk4ydj) or [Matrix](https://matrix.to/#/%23kait2en:matrix.org) to make sure you won't run into issues like kernel regressions.

## So you messed up?

You can mess up DKMS when upgrading the kernel. For example when interrupting DKMS while the new Kernel is booting.
Then you are left with half broken KAIT2EN modules. To repair this, just boot into an older kernel and clean up the new.

```
#replace the kernel version 7.1.3-201.fc44.x86_64 with your own
sudo dracut --force /boot/initramfs-7.1.3-201.fc44.x86_64.img 7.1.3-201.fc44.x86_64 
sudo kernel-install add 7.1.3-201.fc44.x86_64 /lib/modules/7.1.3-201.fc44.x86_64/vmlinuz
sudo grubby --set-default /boot/vmlinuz-7.1.3-201.fc44.x86_64
```

## Murphy's law: We **WILL** mess up!

We are humans. We will mess up at some point. We recommend not to update when
you don't have an external keyboard/mouse around. Like when you are travelling.
But you should always be able to use GRUB to boot into an older kernel anyways.
But be warned that when we mess up, you could loose VHCI devices or WiFi.
And remember we are not paid. We will waste your time and we don't accept complaints.
