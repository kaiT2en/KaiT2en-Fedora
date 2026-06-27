<p align="center">
  <img src="assets/kaiT2en-logo-tr.png" alt="KaiT2en logo" width="240">
</p>

# KaiT2en Fedora

KaiT2en [ˈkaɪ̯zɛn] refers to the Japanese philosophy of "kaizen". Which
means constant small improvements.

Kait2en is using DKMS modules to add T2 Mac driver support to stock Fedora.

It also ships with the latest workarounds to make suspend working and includes
T2 specific apps and tools.

The DKMS modules in this repository build against the currently installed
kernel. This makes it possible for developers to quickly test patches without
the need of recompiling the kernel. Also users can profit quickly from the
latest efforts. Modules and Kernels are updated separately.
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

## Start here

Read the howto documents in order:

1. [Introduction](howto/00-introduction.md)
2. [Get Broadcom firmware from macOS](howto/01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](howto/02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](howto/03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](howto/04-install-kait2en-modules-and-apps.md)

macOS must stay installed. If you want to erase macOS completely, use the regular
T2 Linux documentation instead.

## Working state and remaining work

KaiT2en is meant to provide a usable daily-driver baseline on stock Fedora. Most
hardware works because KaiT2en carries the missing pieces out of tree while the
real fixes are prepared for upstream.

The table below is not just a feature checklist. It also shows where work is
still needed before the KaiT2en patches can become normal upstream Linux
support. Contributions that move these items toward clean upstreamable fixes are
welcome.

| Area | Daily-driver state | Upstreamed | Remaining work |
| --- | --- | --- | --- |
| Audio | Working | No | Provided through `t2bce`. Audio should be split out of BCE internals. |
| Bluetooth | Working | Partial | `brcmfmac` and `hci_bcm4377` need firmware and suspend quirks. |
| Camera | Working | No | Provided through the T2 BCE path. |
| Hybrid graphics | Working | Partial | `amdgpu` and `gmux` behavior needs proper fixes instead of suspend-time workarounds. |
| Keyboard | Working | Yes | Depends on the T2 BCE VHCI path and KaiT2en input drivers. |
| Suspend | Working | No | We install suspend helpers for model-specific `amdgpu` and BCM4377 handling. Which need fixes in drivers. |
| Thunderbolt | Working | Yes | We added further fixes that need upstreaming |
| Touch Bar | Working | Yes | Depends on T2 BCE VHCI which is not upstreamed |
| Touch ID | Not working | No | Needs reverse engineering. |
| T2 AVE | Not working | No | Needs reverse engineering. |
| Trackpad | Working | Yes | Depends on T2 BCE VHCI which is not upstreamed |
| Wi-Fi | Working | Yes | Requires firmware copied from macOS. Firmware handling must stay user-local. |

The largest remaining kernel task is `t2bce` aka `apple-bce`. It should be
broken into smaller upstreamable pieces instead of staying as one large T2
bridge driver. Known areas are DMA which should support scatter-gather support,
mailbox handling, VHCI, audio and suspend/resume ordering. Suspend in particular
needs a fast reverse path.

## Driver naming

KaiT2en renames drivers it maintains because the original Fedora, upstream Linux
and older T2 Linux drivers must be blocked during boot. If KaiT2en used the same
module names, the kernel arguments that block the original drivers would also
block our replacements.

The new names also make logs, `lsmod`, DKMS state and bug reports easier to
read.

| KaiT2en driver | Replaces or carries | Upstream state | Function |
| --- | --- | --- | --- |
| `hid_t2magicmouse` | `hid_magicmouse` with T2 patches | Partial | Apple Magic Mouse, Magic Trackpad and T2 Wellspring trackpad HID support. |
| `t2bce` | `apple-bce` | No | T2 bridge controller, VHCI devices, audio and camera transport. |
| `t2bdrm` | `appletbdrm` | Partial | Touch Bar display DRM device for `react-drm`. |
| `t2gmux` | `apple_gmux` | Partial | GMUX handling on dual-GPU T2 Macs. |
| `t2hid` | `hid_apple` | Partial | Apple HID quirks for T2 keyboards and related internal input devices. |
| `t2mfi_fastcharge` | `apple_mfi_fastcharge` | No | iPhone and iPad fast charging on Apple USB controllers. |
| `t2smc` | `applesmc`, `macsmc` pieces | No | Fan control, battery charge limit, hwmon sensors and RTC through the T2 SMC. |
| `t2thunderbolt` | `thunderbolt` | Partial | Thunderbolt and USB4 controller support on T2 Macs. |
| `t2touchbar_bl` | `hid_appletb_bl` | Yes | Touch Bar backlight handling. |
| `t2touchbar_kbd` | `hid_appletb_kbd` | Yes | Touch Bar keyboard mode handling. |

## Contributing

Contributions are welcome, especially when they move KaiT2en fixes closer to
clean upstream Linux support.

Please keep changes focused. A good contribution should explain what hardware
was tested, which Fedora kernel was used and what changed in the logs or device
behavior.

## License

KaiT2en-owned scripts, howto documents, project text and helper code are MIT
licensed.

Kernel modules, apps and third-party tools may include code with different
origins. Those components keep their own licenses in their directories.
