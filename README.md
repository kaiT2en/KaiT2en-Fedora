<p align="center">
  <img src="assets/kait2en-fedora.jpeg" alt="KaiT2en logo" width="720">
</p>

# KaiT2en Fedora

KaiT2en [ˈkaɪ̯zɛn] refers to the Japanese philosophy of "kaizen". Which
means constant small improvements.

Kait2en is not a standalone distribution. That is by design.
It can be installed on top of stock Fedora or T2Linux Fedora.
It delivers T2 Mac driver support to stock Fedora using out-of-tree
DKMS modules, which are constantly improved for later direct mainline
kernel submission.
This has some major advantages for users and developers alike:

- most users get a working daily driver out of the box. See [*problematic macs*](#problematic-macs)
- driver behaviour is isolated by a clean environment
- you always get the latest vanilla kernel directly from Fedora
- devs don't need to compile the kernel for debugging modules
- by distributing updates through OOTM, devs get instant feedback from users
- we are able to react to regressions instantly

Our goal is upstream to mainline kernel in the first place, but also
to complement the T2linux community by offering an alternative
development workflow. While pre-patched distributions serve a purpose,
managing multiple custom flavors can introduce fragmented workarounds,
which complicates driver developing and testing.

KaiT2en will not work on other distros than Fedora.
In the first place we want a unified clean platform for debugging. We do not
support ports to other distros. We know many of you prefer Arch. But we need
our base to be as conventional as possible to get things sorted.

> [!TIP]
> T2 Linux Fedora can be reverted to vanilla Fedora with KaiT2en on top using
> our revert script in [howto 05](howto/05-revert-t2linux-fedora.md)
> after installation of KaiT2en.

The repository is meant to be used as an offline USB kit. Copy it to a USB
drive, keep that drive connected, and run all commands from the repository root
unless a guide says otherwise.
The setup is intentionally explicit. You will use the terminal, run commands and
know which file was installed where.

## Technical achievements beyond the current T2Linux stack

- `t2bce` replaces apple-bce. It is split into four separate upstreamable modules for core, vhci, (sg-)DMA, audio
- working suspend out of the box for most T2 Macs with working touchbar after resume
- react-drm as replacement for tiny-dfr - content aware touchbar controls
- t2-fan-control is a GUI app for temperature/fan control curves that replaces t2fan-rd
- `t2bce_audio` replaces `aaudio` with stutter-free audio and upstream-friendly UCM support
- ships with a fork of `apple-t2-audio-dsp`. [(Supported devices)](howto/04-install-kait2en-modules-and-apps.md#apple-t2-audio-dsp)
- `t2smc` replaces `applesmc`, adds rtc, hwmon support, SMC sensor support and battery charge limiting
- t2-smc-control is a GUI app for setting battery charge limit and inspecting rtc and real time SMC sensor data
- fixes broken ACPI tables that show as `AE_AML_BUFFER_LIMIT`and `AE_ALREADY_EXISTS` in journal
- always up-to-date vanilla Fedora kernel by nature

## Start Installation from here

Read the howto documents in order:

1. [Introduction to installation](howto/00-introduction.md)
2. [Get Broadcom firmware from macOS](howto/01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](howto/02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](howto/03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](howto/04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](howto/05-revert-t2linux-fedora.md)
7. [Configure GPUs](howto/06-configuring-gpus.md)
8. [How to update](howto/07-updating.md)

## Community

Join the KaiT2en community on [Discord](https://discord.gg/AGfjRk4ydj) or on [Matrix](https://matrix.to/#/%23kait2en:matrix.org)

## Problematic Macs

Most Macs will run just fine out of the box. There are some Macs that have issues on a very low level,
mostly GPU related.

- MacBook Pro A1990 15,1 SMU is different from other MacBooks. Resume is broken when running with dGPU as primary GPU. Users can workaround that as described in [06-configuring-gpus.md](howto/06-configuring-gpus.md)
- Mac Pro 7,1 needs the the Infinity Fabric Link jumpered and Wifi isn't working. Not much info here, since it's a rare bird.
- iMac 27" 5k will only display 4k.
- iMac 20" and 27" show inconsistent GPU behaviour on boot like sporadical black screens.

## Contributing

Contributions are welcome, especially when they move KaiT2en fixes closer to
clean upstream Linux support.

Please keep changes and PR desciptions focused. You may use AI for debugging, but we will notice
slop and we will refuse to review or even merge obvious slop. We are not interested in workarounds.
There is a distinct difference in just making broken things work and fixing things.

#### Remaining work for contributors

We are still using some workarounds to make things work. In long terms this should
be replaced with real fixes that can be upstreamed.

- `t2bce` is our replacement for apple-bce. It is divided into separate modules for 
Core functions, DMA, VHCI and Audio. For upstreaming it needs code review and 
commenting T2 particularities (don't use AI on this).
- We need a OSDW quirk in upstream ACPI/Thunderbolt drivers to get away from kernel param `!Darwin` 
- Macbook 15,1 needs gmux, vgaswitcheroo, amdgpu and maybe even i915 work for the SMU to
survive suspend.
- Mac Pro 7,1 is a rare snowflake and suffers an issue with the Infinity Fabric Link 
- iMac situation is unclear where the dGPU is sporadically not properly intitialized on boot.
Also 5k support remains an issue on 27" iMacs.
- Broadcom 4377 chips need a fix in brcmfmac to work around the firmware refusing D0 to D3cold transition.
- If anyhow possible find a way to make Apple Broadcom chips work without MacOS firmware.
- Get bridgeOS logs from T2 without macOS.
- AVE support needs reverse engineering
- Fingerprint support needs reverse engineering.

## License

KaiT2en-owned scripts, howto documents, project text and helper code are MIT
licensed.

Kernel modules, apps and third-party tools may include code with different
origins. Those components keep their own licenses in their directories.
