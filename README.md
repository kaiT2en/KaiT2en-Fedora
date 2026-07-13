<p align="center">
  <img src="assets/kaiT2en-logo-tr.png" alt="KaiT2en logo" width="240">
</p>

# KaiT2en Fedora

KaiT2en [ˈkaɪ̯zɛn] refers to the Japanese philosophy of "kaizen". Which
means constant small improvements.

Kait2en is not a standalone distribution. By design. 
It delivers T2 Mac driver support to stock Fedora using out-of-tree
DKMS modules, which are constantly improved for later direct mainline
kernel submission. This has some major advantages for users
and developers alike:

- most users get a working daily driver out of the box
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
There is a distinct difference in making broken things work and in
fixing broken things. We want things to be fixed for upstream.

KaiT2en can also be installed on top of existing T2linux.org Fedora kernels.
It blacklists all drivers which we think or know are problematic and replaces
them with its own. Note that a clean Fedora install is preferred because
T2linux Fedora comes with some workarounds and Kernel parameters baked in.

KaiT2en will not work on other distros than Fedora. This was a deliberate choice.
In the first place we want a unified clean platform for debugging. We do not
support ports to other distros. We know many of you prefer Arch. But we need
our base to be as conventional as possible to really get things sorted.

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

KaiT2en is meant to provide a usable daily-driver baseline on stock Fedora.
It carries the missing pieces out of tree while the
real fixes are prepared for upstream.

The table below is not just a feature checklist. It also shows where work is
still needed before the KaiT2en patches can become normal upstream Linux
support. Contributions that move these items toward clean upstreamable fixes are
welcome.

| Area | Daily-driver state | Upstreamed | Remaining work |
| --- | --- | --- | --- |
| Audio | Working | Prepared | - |
| Bluetooth | Working | Partial | `brcmfmac` and `hci_bcm4377` needs suspend quirks. |
| Camera | Working | No | - |
| Hybrid graphics | Working | Partial | `amdgpu` and `gmux` behavior needs proper fixes instead of suspend-time workarounds. |
| Keyboard | Working | Yes | - |
| Suspend | Working | No | We install suspend helpers for model-specific `amdgpu` and BCM4377 handling. Which need fixes in drivers. |
| Thunderbolt | Working | Partial | Some Mac models show pcie ordering/tunnel issues which don't seem to affect operation  |
| Touch Bar | Working | Yes | - |
| Touch ID | Not working | No | Needs reverse engineering. |
| T2 AVE | Not working | No | Needs reverse engineering. |
| Trackpad | Working | Partial | Depends on Asahi patches which need to be upstreamed by them |
| Wi-Fi | Working | Yes | Requires firmware copied from macOS. Firmware handling must stay user-local. |

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
| `t2bce_<module>` | `apple-bce` | No | T2 bridge controller, VHCI devices, DMA and mailbox. |
| `t2bdrm` | `appletbdrm` | Yes | Touch Bar display DRM device for `react-drm`. |
| `t2gmux` | `apple_gmux` | Partial | GMUX handling on dual-GPU T2 Macs. |
| `t2hid` | `hid_apple` | Yes | Apple HID quirks for T2 keyboards and related internal input devices. |
| `t2mfi_fastcharge` | `apple_mfi_fastcharge` | Yes | iPhone and iPad fast charging on Apple USB controllers. |
| `t2smc` | `applesmc`, `macsmc` pieces | No | Fan control, battery charge limit, hwmon sensors and RTC through the T2 SMC. |
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
