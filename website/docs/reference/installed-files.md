# Installed files

KAIT2EN keeps Fedora's vanilla kernel and installs T2 hardware support as a
separate layer. The changes we are doing to your system are documented here to
provide transparency to users and devs.

## Repo location and updater

The guided installer creates a clean git checkout of the `main` branch at:

```text
/usr/local/src/KaiT2en-Fedora
```

**Never change files in this directory manually unless you are told to.**

`kait2en-install` fast-forwards this checkout and runs
`scripts/fedora/install.sh`. It refuses to overwrite local changes or use a
checkout with an unexpected Git remote. Its persistent files are:

```text
/usr/local/bin/kait2en-install
/var/lib/kait2en-installer/state
~/.local/state/kait2en/install.log
```

## Multicall commands

`install.sh` installs a helper script and links the short commands to it. The
command name selects the action, see [Multicall commands](../post-install/multicall-commands.md).

```text
/usr/local/bin/kait2en-multicall
/usr/local/bin/edit-grub          -> kait2en-multicall
/usr/local/bin/update-grub        -> kait2en-multicall
/usr/local/bin/boot-macos         -> kait2en-multicall
/usr/local/bin/update-modules     -> kait2en-multicall
/usr/local/bin/update-apps        -> kait2en-multicall
```

`boot-macos` writes the EFI `BootNext` variable and `update-modules` and
`update-apps` reset the checkout at `/usr/local/src/KaiT2en-Fedora`.

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
| `t2bce_stack` | `t2bce_ave` | Apple T2 AVE HEVC encoder |
| `t2smc` | `t2smc` | Fan, temperature, charge-limit, RTC and SMC power limit access through hwmon |
| `t2bdrm` | `t2bdrm` | Touch Bar DRM display device |
| `t2touchbar` | `t2hid`, `t2touchbar_bl`, `t2touchbar_kbd` | Internal HID quirks, Touch Bar backlight and keyboard mode |
| `hid_t2magicmouse` | `hid_t2magicmouse` | Internal trackpad support with the required Asahi patches |
| `t2_precision_trackpad` | `t2_precision_trackpad` | Force Touch trackpad driver with dynamic surface geometry and a separate Force Click event |
| `t2_trackpad_actuator` | `t2_trackpad_actuator` | T2 Force Touch actuator transport |
| `t2mfi_fastcharge` | `t2mfi_fastcharge` | Fast-charge control for Apple MFi devices |
| `t2gmux` | `t2gmux` | GMUX handling on dual-GPU Macs |
| `t2thunderbolt` | `t2thunderbolt` | Thunderbolt power-management ordering and T2 PCI quirks |
| `t2smp` | `t2smp` | Defers secondary CPU hotplug to avoid firmware-sensitive resume delays |
| `t2sbs` | `t2sbs` | Smart Battery driver that refreshes the full charge capacity and reports the charge relative to it, replaces `sbs` |
| `linux-apfs-rw` | `apfs` | Read-only access to the macOS APFS container, vendored from upstream |

The installer also writes
`/etc/kernel/install.d/39-kait2en-dkms-cleanup.install`. The hook removes stale
DKMS build state before a kernel installation is retried.

On the MacBookPro15,1, MacBookPro16,1, MacBookPro16,4, iMac20,1 and iMac20,2,
the installer also builds the AMDGPU module with the patches from
`patches/runtime/gpu-runtime-pm` (hybrid runtime PM, 5K iMac panel support,
the Navi 14 memory clock cap, the Apple Navi power limit range and a revert
of the 7.2 ASPM host-link check)
and installs it for the running kernel at:

```text
/usr/lib/modules/<kernel>/updates/kait2en-gpu-runtime-pm/amdgpu.ko.xz
```

The corresponding modprobe configuration is installed as
`/usr/lib/modprobe.d/kait2en-gpu-runtime-pm.conf`.

Unlike the T2 modules above, this patched AMDGPU module is not built by DKMS.
Instead, the installer copies the build script and the patches to
`/usr/local/libexec/kait2en/gpu-runtime-pm/` and writes
`/etc/kernel/install.d/45-kait2en-gpu-runtime-pm.install`. When Fedora installs
a new kernel, the hook builds the module for it after `40-dkms.install` and
before `50-dracut.install`, and logs to `/var/log/kait2en-gpu-runtime-pm.log`.
A failed build never aborts the kernel update. When a kernel is removed, the
hook deletes its module. To rebuild for a specific kernel by hand:

```bash
sudo /usr/local/libexec/kait2en/gpu-runtime-pm/install-gpu-runtime-pm.sh install <kernel>
```

Running the installer again refreshes the copy with the current patches.

## Kernel arguments

`install-kernel-args.sh` updates every installed kernel entry with
`grubby --update-kernel=ALL`. Inspect the effective arguments with:

```bash
grubby --info=DEFAULT
```

KAIT2EN adds these arguments to every installed kernel through `grubby`:

```text
amdgpu.aspm=1
intel_iommu=on
iommu=pt
pm_async=off
brcmfmac.p2pon=0
pcie_aspm=force
pcie_aspm.policy=powersave
pcie_ports=compat
mem_sleep_default=deep
initcall_blacklist=cmos_init,magicmouse_driver_init
module_blacklist=acpi_tad,applesmc,macsmc,hid_apple,hid_appletb_bl,hid_appletb_kbd,hid_magicmouse,appletbdrm,apple_bce,apple_mfi_fastcharge,apple_gmux,sbs
```

On iMacs and the iMac Pro, `pcie_aspm=force` is left out, because their
Ethernet controllers do not work with forced ASPM. On the MacBookPro16,1,
`amdgpu.aspm=1`, `pcie_aspm=force` and `pcie_aspm.policy=powersave` are left
out while its Navi 14 drops off the PCIe bus under load.

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

The installer and the independent `t2-dsp` package deploy all model-specific
graphs and FIR data. udev and WirePlumber select the matching profile at runtime.

```text
/usr/share/t2-dsp/profiles/<profile>/
/usr/share/wireplumber/wireplumber.conf.d/51-t2-dsp.conf
/usr/share/wireplumber/scripts/t2-default-output.lua
/usr/share/pipewire/pipewire.conf.d/50-kait2en-quantum.conf
/usr/lib/udev/rules.d/89-t2-dsp.rules
/usr/libexec/t2-dsp/package-actions
/usr/share/licenses/t2-dsp/
```

Unlisted models receive no DSP graph at runtime. Former generated `/etc`
fragments are backed up under `/var/lib/kait2en/migration/t2-dsp/` with ownership
receipts under `/var/lib/kait2en/ownership/`. Removal cleans verified backups.
Migration errors
are collected in `/var/log/t2-dsp-install.log` and the installer summary.
See [Audio DSP](../post-install/audio-dsp.md) for the supported-model table,
audio behavior and diagnostics.

## System configuration and services

| Path | Purpose |
| --- | --- |
| `/etc/systemd/system/kait2en-suspend.service` | Calls the suspend helper before `sleep.target` and again after resume |
| `/usr/local/libexec/kait2en/kait2en-suspend.sh` | Handles the BCM4377 suspend workaround described below |
| `/etc/systemd/system/kait2en-macos-mount.service` | Mounts the readable macOS volumes read-only under `/media/macOS` at boot |
| `/usr/local/libexec/kait2en/kait2en-macos-mount` | Finds the readable volumes in the internal APFS container and mounts them as subdirectories by role |
| `/etc/systemd/system/t2-services-suspend.service` | Runs feature sleep/resume hooks around sleep |
| `/usr/local/libexec/t2-services/t2-ncm-sleep` | Runs installed feature hooks before sleep and after resume |
| `/usr/local/libexec/t2-services/sleep.d/t2-ave` | Closes/reopens sessions only when the AVE daemon is running |
| `/etc/systemd/system/kait2en-t2-remote.service` | Loads `t2bce_ave` and runs `t2remote`, which holds the bridgeOS services Linux uses, currently `com.apple.aveservice` for the video encoder |
| `/usr/local/bin/t2remote` | RemoteXPC service manager; `t2remote status` shows the live services. Listens on `/run/t2remote.sock` |
| NetworkManager profile `Apple T2 Bridge` | IPv6 link-local connection to the T2 over its internal CDC-NCM interface, bound to the interface's MAC address. Replaces the earlier `t2_ncm` udev rule |
| `/etc/modprobe.d/kait2en-silent-blacklist.conf` | Silently ignores attempts to load drivers replaced by KAIT2EN modules |
| `/usr/share/plymouth/themes/kait2en/` | macOS-style boot splash with a KAIT2EN logo |
| `/etc/plymouth/plymouthd.conf` | Selects the KAIT2EN theme, pins `DeviceScale=1` and sets `UseSimpledrm=0` so the splash does not resize when i915 replaces the 2880x1800 EFI framebuffer |
| `/usr/share/pixmaps/kait2en-gdm-logo.png` | White and red KAIT2EN logo shown by GDM |
| `/etc/dconf/db/gdm.d/00-kait2en` | Configures the GDM logo and solid black login background |
| `/usr/share/backgrounds/kait2en/gdm-black.png` | Black GDM background image |
| `/usr/share/gnome-shell/gnome-shell-theme.gresource` | GNOME Shell theme patched so the GDM and lock-screen shield render black |
| `/etc/dracut.conf.d/90-kait2en-input.conf` | Keeps the internal keyboard drivers, `t2smc` and `t2sbs` in initramfs images built during kernel updates |
| `/boot/initramfs-<running-kernel>.img` | Rebuilt by Dracut after modules and ACPI handling are complete |

`kait2en-suspend.service` is enabled on every installation. Before suspend, it
checks for a Broadcom PCI device with vendor ID `0x14e4` and device ID `0x5f69`,
`0x5f71`, `0x5f72` or `0x5fa0`. If one is present, it unloads `brcmfmac_wcc`,
  `brcmfmac` and `hci_bcm4377` in that order. After resume it loads `brcmfmac`
  and `brcmfmac_wcc`, waits five seconds, then loads `hci_bcm4377`.

Without a BCM4377, it checks for the BCM4350 WLAN chip (device ID `0x43a3`).

If neither controller is present, the service logs that the fix is not needed
and does not unload a module. State files for modules successfully unloaded by
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

`kait2en-macos-mount.service` is enabled on every installation. At boot the
helper loads `apfs` and probes each volume of every internal APFS partition
read-only in turn. Every readable volume is mounted as a subdirectory of the
single `/media/macOS` directory, named by the role detected from its contents,
so the whole macOS install is browsed from one place:

| Mount point | Volume | Detected by |
| --- | --- | --- |
| `/media/macOS/System` | System | `System/Library/CoreServices/SystemVersion.plist` |
| `/media/macOS/Preboot` | Preboot | `*/restore/BuildManifest.plist` or `*/cryptex1` |
| `/media/macOS/iSCPreboot` | iSCPreboot | `nvram.plist` with a `Firmware/` directory |
| `/media/macOS/Recovery` | Recovery | signed boot objects `*/*.im4m` |

The probe runs on every boot because the volume order differs between Macs and
macOS versions. The macOS Data volume is encrypted and stays unmounted, and the
VM volume is left out.

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

## Applications

`t2-fan-control`, `t2-smc-control`, `t2-power-explorer`, `t2-debug`,
`t2-zone-power`, `t2-journal`, `kait2en-touchbar` with Touch Bar Settings, `kernel-builder`, and
`t2-power-tune` are installed system-wide under `/usr/local`. `kait2en-touchbar` is optional and
only installed when selected at the start of the installation.

MacBookPro15,1, MacBookPro16,1 and MacBookPro16,4 get
`t2-hybrid-gpu-control`; other MacBook Pro models with Intel and AMD display
devices get `t2-dgpu-control`.
All KAIT2EN desktop applications use the shared header wordmark at
`/usr/local/share/kait2en/kait2en-wordmark.png`.

| Application | Installed files |
| --- | --- |
| T2 Fan Control | `/usr/local/bin/t2-fancontrol-gtk`, `/usr/local/share/applications/org.t2fancontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2fancontrol.gtk.svg`, `/usr/local/lib/systemd/system/t2-fancontrol.service` |
| T2 SMC Control | `/usr/local/bin/t2-smc-control`, `/usr/local/share/applications/org.t2smccontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2smccontrol.gtk.svg`, `/usr/local/lib/systemd/system/t2-smc-gaugingd.service`, `/var/lib/t2-smc-control/gauging` |
| T2 Power Explorer | `/usr/local/bin/t2-power-explorer`, `/usr/local/libexec/t2-power-explorer-status`, `/usr/local/share/applications/org.t2powerexplorer.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2powerexplorer.gtk.svg`, `/usr/share/polkit-1/actions/org.t2powerexplorer.policy` |
| T2 Journal | `/usr/local/bin/t2journal` |
| T2 Zone Power | `/usr/local/bin/t2-zonepower-gtk`, `/usr/local/share/applications/org.t2zonepower.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2zonepower.gtk.svg`, `/usr/local/lib/systemd/system/t2-zonepowerd.service` |
| T2 Kernel Builder | `/usr/local/bin/t2-kernel-builder`, `/usr/local/libexec/t2-kernel-builder-cleanup`, `/usr/local/libexec/t2-kernel-builder/build.sh`, `/usr/local/libexec/t2-kernel-builder/configs/*.config`, `/usr/local/share/applications/org.t2kernelbuilder.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2kernelbuilder.gtk.svg`, `/usr/local/share/polkit-1/actions/org.t2kernelbuilder.gtk.policy` |
| T2 Power Tune | `/usr/local/bin/t2-power-tune`, `/usr/local/libexec/t2-power-tune-helper`, `/usr/local/libexec/t2-power-tune-status`, `/usr/local/share/applications/org.t2powertune.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2powertune.gtk.svg`, `/usr/share/polkit-1/actions/org.t2powertune.policy` |
| T2 Force Click | `/usr/local/bin/t2-force-click`, `/usr/local/share/applications/org.t2forceclick.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2forceclick.gtk.svg`, `/usr/local/lib/systemd/system/t2-force-click.service` |
| T2 Debug | `/usr/local/bin/t2-debug`, `/usr/local/bin/t2debug`, `/usr/local/share/applications/org.t2debug.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2debug.gtk.svg` |
| T2 Touch Bar | `/usr/local/bin/kait2en-touchbar`, `/usr/local/lib/systemd/user/kait2en-touchbar.service`, `/usr/local/lib/systemd/system/kait2en-touchbar-attach.service`, `/usr/local/lib/udev/rules.d/99-zz-kait2en-touchbar.rules`, `/etc/kait2en/touchbar.toml`, `/usr/local/share/licenses/kait2en-touchbar/THIRD-PARTY-NOTICES.md` |
| Touch Bar Settings | `/usr/local/bin/kait2en-touchbar-settings`, `/usr/local/share/applications/org.kait2entouchbar.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.kait2entouchbar.gtk.svg` |
| T2 Hybrid GPU Control | `/usr/local/bin/t2-hybrid-gpu-control`, `/usr/local/libexec/t2-hybrid-gpu-control-helper`, `/usr/local/libexec/t2-hybrid-gpu-control-status`, `/usr/local/share/applications/org.t2hybridgpucontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2hybridgpucontrol.gtk.svg`, `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.policy`, `/usr/share/polkit-1/actions/org.t2hybridgpucontrol.gtk.status.policy`, `/etc/environment.d/90-kait2en-gsk-renderer.conf` |
| T2 GPU Control | `/usr/local/bin/t2-dgpu-control`, `/usr/local/libexec/t2-dgpu-control-helper`, `/usr/local/libexec/t2-dgpu-control-status`, `/usr/local/share/applications/org.t2dgpucontrol.gtk.desktop`, `/usr/local/share/icons/hicolor/scalable/apps/org.t2dgpucontrol.gtk.svg`, `/usr/local/lib/systemd/system/kait2en-dgpu-off.service`, `/usr/local/lib/systemd/system/kait2en-dgpu-suspend.service`, `/usr/local/lib/systemd/system/kait2en-amdgpu-profile.service`, `/usr/local/lib/systemd/system/kait2en-amdgpu-profile-resume.service`, `/usr/share/polkit-1/actions/org.t2dgpucontrol.gtk.policy`, `/usr/share/polkit-1/actions/org.t2dgpucontrol.gtk.status.policy` |

T2 Fan Control's service starts immediately and persists fan curves across
boot and resume. T2 Hybrid GPU Control does not install system services.
T2 GPU Control enables its units only when the corresponding options are
applied in the app. Both privileged helpers validate the GPU layout and accept
only the fixed operations exposed by their UI.

T2 Force Click persists its settings at `/etc/t2-force-click/config.txt`; its
root daemon listens at `/run/t2-force-click/daemon.sock`, which is runtime
state and disappears at shutdown. While the driver is loaded, its user-facing
controls are:

```text
/sys/module/t2_precision_trackpad/parameters/click_strength
/sys/module/t2_precision_trackpad/parameters/force_click_threshold_percent
/sys/module/t2_precision_trackpad/parameters/force_click_enabled
```

## T2 trackpad actuator playback

`t2_trackpad_actuator` exposes the DTrace-captured Taptic Engine click
waveforms to privileged local software. A third-party helper can play the
light or firm impulse by writing one of these values:

```sh
printf light | sudo tee /sys/module/t2_trackpad_actuator/parameters/play
printf firm | sudo tee /sys/module/t2_trackpad_actuator/parameters/play
```

The interface accepts no raw waveform bytes. `light` and `firm` select the
captured reports used by the precision trackpad driver. Callers should trigger
individual application events and rate-limit repeated playback themselves.
The parameter is write-only and requires the module to be loaded and bound to
the T2 actuator interface.

Each report takes about 6 ms over the T2 USB transport. While a waveform still
plays, the actuator stalls the next report. The module then resends it once,
as macOS does, so a stall only shows up in the kernel log when the retry
fails as well. Shorter waveforms allow denser sequences.

For diagnostics, root can compose a waveform through the `probe_*` parameters
and play it with `test_fire=9`. Every amplitude is capped at 70, the
`AmplitudeMax` the actuator reports to macOS. macOS itself sends plain
Gaussian clicks with amplitudes between 9 and 38.

T2 Power Tune reads package C-state residency and exposes PCIe ASPM, runtime
power management, LTR ignore, and additional power tunables. The optional
`/etc/systemd/system/kait2en-power-tune.service` is created by the app only when
the user chooses persistent settings. Its runtime selection cache is stored at
`/run/t2-power-tune/items.json` and disappears on reboot.

T2 Journal stores the last successfully parsed BridgeOS log snapshot and a
cache of recently discovered RemoteXPC ports for each desktop user:

```text
~/.local/state/t2-journal/bridgeos.jsonl
~/.local/state/t2-journal/remote-ports
```

`$XDG_STATE_HOME` replaces `~/.local/state` when it is set. The snapshot is
created on the first query or explicit refresh and is atomically replaced only
after a successful download and parse. See [T2 Journal](../post-install/kait2en-applications.md#t2-journal)
for the required network setup.

T2 Kernel Builder keeps downloaded sources, build trees and completed-build
markers below `$XDG_CACHE_HOME/t2-kernel-builder/build` (normally
`~/.cache/t2-kernel-builder/build`) and its saved UI state below
`$XDG_CONFIG_HOME/t2-kernel-builder` (normally `~/.config/t2-kernel-builder`).
Kernels installed through the app add their normal files below `/boot` and
`/lib/modules/<kernel>/`; package-managed builds are installed through DNF.

The installer builds all KaiT2en Rust apps and services into one shared Cargo
target directory at `~/.cache/kait2en/cargo-target` of the user who invoked
`sudo`. The apps share their GTK dependencies there, and updates rebuild only
what changed. Deleting the directory is safe. The next installer run rebuilds
it.
