# How to update

[Automatic installation](automatic-installation.md)

Manual installation:

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md)
8. [How to update](07-updating.md) (you are here)

Previous: [Configure GPUs](06-configuring-gpus.md) | [Back to README](../README.md)

KaiT2en is meant to disappear. It's part of the concept. It serves the purpose
of upstreaming code fixes. Every module and fix that gets upstreamed will disappear from
the repo. Until we are left with a few T2 specific apps and other things that can't be upstreamed.
Maybe it will survive as a collection of helpful apps for T2 Macs.

As long as this isn't the case, we need to update the Fedora kernel and our modules and apps.
The question how to do that has kept us a bit busy. We could go COPR and offer .rpm.
But the project is yet too small to fund itself.

## Updating an automatically installed system

Open a terminal and run:

```bash
kait2en-install
```

This updates the KaiT2en Git checkout and runs the regular project installer.
Review its output and reboot after it completes successfully.

## Updating KaiT2en code manually

For now we presume you just git pull the latest commits to update modules, services and apps.
We may switch to versioned releases in the near future, when the amount of everyday
code changes has settled.
It's recommended to run `install.sh` again on updates. The script will wipe obsolete code
and install the latest versions automatically. So if you missed to update in a long time,
be sure to run `install.sh`.
If you always keep track with the latest changes on [Discord](https://discord.gg/AGfjRk4ydj) or [Matrix](https://matrix.to/#/%23kait2en:matrix.org) and you know what
you're doing, you can use the single scripts. Like when you know that only kernel parameters
changed, then you would only run `install-kernel-args.sh`.

## Updating Fedora

You just update Fedora like everyone else. DKMS will notice and recompile our modules against the
latest kernel. It's always worth visiting the [KaiT2en community on Discord](https://discord.gg/AGfjRk4ydj) or [Matrix](https://matrix.to/#/%23kait2en:matrix.org) to make sure you won't run into issues like kernel regressions.

## Murphy's law: We **WILL** mess up!

We are humans. We will mess up at some point. We recommend not to update when
you don't have an external keyboard/mouse around. Like when you are travelling.
But you should always be able to use GRUB to boot into an older kernel anyways.
But be warned that when we mess up, you could loose VHCI devices or WiFi.
And remember we are not paid. We will waste your time and we don't accept complaints.

Previous: [Configure GPUs](06-configuring-gpus.md) | [Back to README](../README.md)
