# Installed files

KAIT2EN keeps Fedora's vanilla kernel and installs T2 hardware support as a
separate layer. The changes we are doing to your system are documented here to
provide transparency to users and devs.

## Repo location and updater

The guided installer creates a clean checkout of the `main` branch at:

```text
/usr/local/src/KaiT2en-Fedora
```

`kait2en-install` fast-forwards this checkout and runs
`scripts/fedora/install.sh`. It refuses to overwrite local changes or use a
checkout with an unexpected Git remote. Its persistent files are:

```text
/usr/local/bin/kait2en-install
/var/lib/kait2en-installer/state
~/.local/state/kait2en/install.log
```

## Kernel modules

The installer copies each DKMS source package to `/usr/src/<name>-<version>/`,
registers it and builds it for the running Fedora kernel. DKMS keeps
its build state below `/var/lib/dkms/` and rebuilds the modules for later
kernel updates. `modinfo -n <module>` prints the installed kernel object path.

| DKMS source | Installed module | Purpose |
| --- | --- | --- |
| `t2bce_stack` | `t2bce_dma` | Shared DMA queue engine for T2 BCE clients |
| `t2bce_stack` | `t2bce_core` | T2 bridge PCI device, mailbox, power management and transport |
| `t2bce_stack` | `t2bce_vhci` | Virtual USB host for internal T2 input devices |
| `t2bce_stack` | `t2bce_audio` | Apple T2 audio driver |
| `t2smc` | `t2smc` | Fan, temperature, charge-limit and RTC access through hwmon |
| `t2bdrm` | `t2bdrm` | Touch Bar DRM display device |
| `t2touchbar` | `t2hid`, `t2touchbar_bl`, `t2touchbar_kbd` | Internal HID quirks, Touch Bar backlight and keyboard mode |
| `hid_t2magicmouse` | `hid_t2magicmouse` | Internal trackpad support with the required Asahi patches |
| `t2mfi_fastcharge` | `t2mfi_fastcharge` | Fast-charge control for Apple MFi devices |
| `t2gmux` | `t2gmux` | GMUX handling on dual-GPU Macs |
| `t2thunderbolt` | `t2thunderbolt` | Thunderbolt power-management ordering and T2 PCI quirks |
| `t2smp` | `t2smp` | Defers secondary CPU hotplug to avoid firmware-sensitive resume delays |

The installer also writes
`/etc/kernel/install.d/39-kait2en-dkms-cleanup.install`. The hook removes stale
DKMS build state before a kernel installation is retried.

On the MacBookPro15,1, the app installer also builds the AMDGPU and Intel HDA
modules with the hybrid runtime-PM patches and installs them for the running
kernel at:

```text
/usr/lib/modules/<kernel>/updates/kait2en-gpu-runtime-pm/amdgpu.ko.xz
/usr/lib/modules/<kernel>/updates/kait2en-gpu-runtime-pm/snd-hda-intel.ko.xz
```

The corresponding modprobe and dracut configuration is installed as
`/usr/lib/modprobe.d/kait2en-gpu-runtime-pm.conf` and
`/etc/dracut.conf.d/90-kait2en-gpu-runtime-pm.conf`.

Unlike the T2 modules above, these patched AMDGPU and HDA modules are not built
by DKMS. After a Fedora kernel update, boot the new kernel before rebuilding
them for its exact release:

```bash
cd /usr/local/src/KaiT2en-Fedora
sudo ./scripts/fedora/install-gpu-runtime-pm.sh
sudo reboot
```

The script installs the modules for the running kernel and rebuilds that
kernel's initramfs.

## Kernel arguments

`install-kernel-args.sh` updates every installed kernel entry with
`grubby --update-kernel=ALL`. Inspect the effective arguments with:

```bash
grubby --info=DEFAULT
```

KAIT2EN adds these arguments to every installed kernel through `grubby`:

```text
intel_iommu=on
iommu=pt
pm_async=off
brcmfmac.p2pon=0
pcie_aspm=force
pcie_aspm.policy=powersave
pcie_ports=native
pci=noaer
mem_sleep_default=deep
initcall_blacklist=cmos_init,magicmouse_driver_init
module_blacklist=acpi_tad,applesmc,macsmc,hid_apple,hid_appletb_bl,hid_appletb_kbd,hid_magicmouse,appletbdrm,apple_bce,apple_mfi_fastcharge,apple_gmux
```

On every T2 Mac, the installer enables HuC firmware loading for the Intel GPU:

```text
i915.enable_guc=2
```

The value is a bitmask: `2` selects. HuC is
used by Intel media workloads, including HEVC operations.

KAIT2EN does not install a custom kernel or a separate GRUB configuration file.

## Audio configuration

The ALSA UCM profiles define the T2 speaker, microphone and headset paths:

```text
/usr/share/alsa/ucm2/AppleT2/HiFi-x2.conf
/usr/share/alsa/ucm2/AppleT2/HiFi-x4.conf
/usr/share/alsa/ucm2/AppleT2/HiFi-x6.conf
/usr/share/alsa/ucm2/conf.d/AppleT2x2/AppleT2x2.conf
/usr/share/alsa/ucm2/conf.d/AppleT2x4/AppleT2x4.conf
/usr/share/alsa/ucm2/conf.d/AppleT2x6/AppleT2x6.conf
```

The installer deploys model-specific FIR filters and a generated WirePlumber
rule on these models: `MacBookAir8,1`, `MacBookAir8,2`, `MacBookAir9,1`,
`MacBookPro15,1`, `MacBookPro15,2`, `MacBookPro15,4`, `MacBookPro16,1`, `MacBookPro16,2`,
`MacBookPro16,3` and `MacBookPro16,4`.

```text
/usr/share/kait2en/audio-dsp/<profile>/
/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf
```

Every other model exits the DSP step without creating these files.
See [Audio DSP](../post-install/audio-dsp.md) for the supported-model table,
audio behavior and diagnostics.

## System configuration and services

| Path | Purpose |
| --- | --- |
| `/etc/systemd/system/kait2en-suspend.service` | Calls the suspend helper before `sleep.target` and again after resume |
| `/usr/local/libexec/kait2en/kait2en-suspend.sh` | Handles the BCM4377 suspend workaround described below |
| `/etc/udev/rules.d/90-kait2en-t2-network.rules` | Renames the internal T2 debug interface to `t2_ncm` and excludes it from NetworkManager |
| `/etc/modprobe.d/kait2en-silent-blacklist.conf` | Silently ignores attempts to load drivers replaced by KAIT2EN modules |
| `/usr/share/plymouth/themes/kait2en/` | macOS-style boot splash with a KAIT2EN logo |
| `/usr/share/pixmaps/kait2en-gdm-logo.png` | White and red KAIT2EN logo shown by GDM |
| `/etc/dconf/db/gdm.d/00-kait2en` | Configures the GDM logo and solid black login background |
| `/usr/share/backgrounds/kait2en/gdm-black.png` | Black GDM background image |
| `/usr/share/gnome-shell/gnome-shell-theme.gresource` | GNOME Shell theme patched so the GDM and lock-screen shield render black |
| `/etc/dracut.conf.d/90-kait2en-input.conf` | Keeps the internal keyboard drivers in initramfs images built during kernel updates |
| `/boot/initramfs-<running-kernel>.img` | Rebuilt by Dracut after modules and ACPI handling are complete |

`kait2en-suspend.service` is enabled on every installation. Before suspend, it
checks for a Broadcom PCI device with vendor ID `0x14e4` and device ID `0x5f69`,
`0x5f71`, `0x5f72` or `0x5fa0`. If one is present, it unloads `brcmfmac_wcc`,
  `brcmfmac` and `hci_bcm4377` in that order. After resume it loads `brcmfmac`
  and `brcmfmac_wcc`, waits five seconds, then loads `hci_bcm4377`.

If the controller is absent, the service logs that the fix is not needed and
does not unload a module. State files for modules successfully unloaded by
the helper exist only until resume below `/run/kait2en-suspend/`.

The installer checks the running kernel log for two known Apple ACPI firmware
errors. It only deploys an override when the error is present and the generated
table passes validation with `iasl`:

```text
/usr/local/lib/firmware/acpi/*.aml
/usr/local/lib/firmware/acpi/.kait2en-*.sha256
/etc/dracut.conf.d/t2-acpi-fix.conf
/var/backups/t2-acpi-fix/<timestamp>/
```

The backup contains every managed file that existed before deployment and a
manifest. KAIT2EN only replaces or removes an ACPI table when its ownership
marker contains the table's current SHA-256 checksum. Existing unmarked tables
and modified managed tables are left unchanged and reported as conflicts.

## Apple firmware from the installer USB drive

The guided installer copies Apple's own firmware for this Mac from the USB drive
into the installed system, renamed to the file names the Linux drivers ask for:

```text
/usr/lib/firmware/brcm/brcmfmac<chip>-pcie.apple,<board>*
```

Macs with the BCM4377 PCIe Bluetooth controller (`14e4:5fa0`) additionally get
the two Bluetooth blobs. Every other T2 Mac drives Bluetooth over UART and gets
nothing here:

```text
/usr/lib/firmware/brcm/brcmbt<chip><stepping>-apple,<board>[-<vendor>].bin
/usr/lib/firmware/brcm/brcmbt<chip><stepping>-apple,<board>[-<vendor>].ptb
```

The exact names are taken from what the driver asked for in the kernel log. The
outcome of the Bluetooth step is recorded in the installed system at:

```text
/var/log/kait2en/bluetooth-firmware.log
```

## Desktop applications

`t2-fan-control`, `t2-smc-control`, `t2-power-explorer`, `t2-cpu-control`, and
`t2-power-tune` are installed system-wide under `/usr/local`. The
MacBookPro15,1 gets `t2-hybrid-gpu-control`; other MacBook Pro models with Intel
and AMD display devices get `t2-dgpu-control`.
All Kait2en desktop applications use the shared header wordmark at
`/usr/local/share/kait2en/kait2en-wordmark.png`.

| Application | Installed files |
| --- | --- |
| T2 Fan Control | `/usr/local/bin/t2-fancontrol-gtk`, `/usr/local/share/applications/org.t2fancontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2fancontrol.gtk.svg`, `/usr/local/lib/systemd/system/t2-fancontrol.service` |
| T2 SMC Control | `/usr/local/bin/t2-smc-control`, `/usr/local/share/applications/org.t2smccontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2smccontrol.gtk.svg` |
| T2 Power Explorer | `/usr/local/bin/t2-power-explorer`, `/usr/local/libexec/t2-power-explorer-status`, `/usr/local/share/applications/org.t2powerexplorer.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2powerexplorer.gtk.svg`, `/usr/share/polkit-1/actions/org.t2powerexplorer.policy` |
| T2 CPU Control | `/usr/local/bin/t2-cpu-control`, `/usr/local/libexec/t2-cpu-control-helper`, `/usr/local/libexec/t2-cpu-control-status`, `/usr/local/libexec/t2-cpu-kernel-benchmark`, `/usr/local/lib/systemd/system/t2-cpu-control.service`, `/usr/local/lib/systemd/system-sleep/t2-cpu-control`, `/usr/local/share/applications/org.t2cpucontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2cpucontrol.gtk.svg`, `/usr/share/polkit-1/actions/org.t2cpucontrol.policy` |
| T2 Power Tune | `/usr/local/bin/t2-power-tune`, `/usr/local/libexec/t2-power-tune-helper`, `/usr/local/libexec/t2-power-tune-status`, `/usr/local/share/applications/org.t2powertune.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2powertune.gtk.svg`, `/usr/share/polkit-1/actions/org.t2powertune.policy` |
| T2 Hybrid GPU Control | `/usr/local/bin/t2-hybrid-gpu-control`, `/usr/local/libexec/t2-hybrid-gpu-control-helper`, `/usr/local/libexec/t2-hybrid-gpu-control-status`, `/usr/local/share/applications/org.t2hybridgpucontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2hybridgpucontrol.gtk.svg`, `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.policy`, `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.status.policy` |
| T2 GPU Control | `/usr/local/bin/t2-dgpu-control`, `/usr/local/libexec/t2-dgpu-control-helper`, `/usr/local/libexec/t2-dgpu-control-status`, `/usr/local/share/applications/org.t2dgpucontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2dgpucontrol.gtk.svg`, `/usr/local/lib/systemd/system/kait2en-dgpu-off.service`, `/usr/local/lib/systemd/system/kait2en-dgpu-suspend.service`, `/usr/local/lib/systemd/system/kait2en-amdgpu-profile.service`, `/usr/local/lib/systemd/system/kait2en-amdgpu-profile-resume.service`, `/usr/share/polkit-1/actions/org.t2dgpucontrol.gtk.policy`, `/usr/share/polkit-1/actions/org.t2dgpucontrol.gtk.status.policy` |

T2 Fan Control's service starts immediately and persists fan curves across
boot and resume. T2 Hybrid GPU Control does not install system services.
T2 GPU Control enables its units only when the corresponding options are
applied in the app. Both privileged helpers validate the GPU layout and accept
only the fixed operations exposed by their UI.

T2 Power Tune reads package C-state residency and exposes PCIe ASPM, runtime
power management, LTR ignore, and additional power tunables. The optional
`/etc/systemd/system/kait2en-power-tune.service` is created by the app only when
the user chooses persistent settings. Its runtime selection cache is stored at
`/run/t2-power-tune/items.json` and disappears on reboot.

`react-drm` is installed for the desktop user only when the DMI product name is
one of `MacBookPro15,1`, `MacBookPro15,2`, `MacBookPro15,3`, `MacBookPro15,4`,
`MacBookPro16,1`, `MacBookPro16,2`, `MacBookPro16,3` or `MacBookPro16,4`:

```text
~/react-drm/
~/.config/systemd/user/react-drm.service
/etc/udev/rules.d/99-react-drm.rules
```

The installer adds that user to the `video` and `input` groups. When the active
desktop reported by the user's systemd environment contains `gnome`, it also
installs Window Monitor Pro below `~/.local/share/gnome-shell/extensions/` for
application-aware controls. Every other DMI product name exits this step before
installing react-drm dependencies or files.
