# Install KAIT2EN modules and apps

Previous: [Install Broadcom firmware on Fedora](02-install-broadcom-firmware.md) | [Installation introduction](../../introduction.md)

## Install.sh

Install.sh will install DKMS modules, scripts and apps by running a stack
of sub-scripts.

Run this command from the KAIT2EN repository root on your Fedora system:

```bash
sudo bash ./scripts/fedora/install.sh
```

This runs all required installation steps in order.

It will take a few minutes to compile the modules and apps against the kernel.
Stay around to enter needed confirmations while the script is running.
Please reboot after the script completed without errors.

```bash
sudo reboot
```

## Modules

KAIT2EN renames drivers it maintains because the original Fedora and older
T2 Linux drivers must be blocked during boot. If KAIT2EN used the same
module names, the kernel arguments that block the original drivers would also
block our replacements.

The new names also make logs, `lsmod`, DKMS state and bug reports easier to
read. The list below shows which modules are blacklisted and replaced by their
KAIT2EN equivalents:

T2 Linux driver &#8594; KAIT2EN driver

`hid_magicmouse` &#8594; `hid_t2magicmouse` contains Asahi trackpad patches

`apple-bce` &#8594; `t2bce_<module>` T2 bridge, audio, VHCI devices, DMA and mailbox

`appletbdrm` &#8594; `t2bdrm` Touch Bar display DRM device

`apple_gmux` &#8594; `t2gmux` GMUX handling on dual-GPU Macs

`hid_apple` &#8594; `t2hid` Apple HID quirks for internal input devices

`apple_mfi_fastcharge` &#8594; `t2mfi_fastcharge` fast charging on Apple USB controllers.

`applesmc` &#8594; `t2smc` battery charge limit, hwmon sensors and RTC through the T2 SMC.

`hid_appletb_bl` &#8594; `t2touchbar_bl` Touch Bar backlight handling.

`hid_appletb_kbd` &#8594; `t2touchbar_kbd` Touch Bar keyboard mode handling.

## Scripts and services

Below you will find all scripts and apps that are installed when running `install.sh`.
You dont't need to run them manually. You can do that when you only want
to install updates partially. 
Also devs please note [how to enable debugging](#kernel-debug-logs) on the very bottom of this document.

### Automatic ACPI firmware fixes

Before rebuilding the initramfs, the installer checks for two known ACPI
problems on Intel T2 Macs:

- CpuSSDT tries to load SSDT sub-tables already loaded by Linux. The fix
  pre-initializes `\SDTL` so `GCAP` skips them and avoids `AE_ALREADY_EXISTS`.
- a DSDT `_OSC` buffer overflow reported as `AE_AML_BUFFER_LIMIT`

The autofix builds the matching override from the running Mac's ACPI tables and
validates it with `iasl`. A table is not installed if validation fails.

Successful overrides and their Dracut configuration are written to:

```text
/usr/local/lib/firmware/acpi/*.aml
/etc/dracut.conf.d/t2-acpi-fix.conf
```

Files replaced during deployment are backed up below
`/var/backups/t2-acpi-fix/<timestamp>/`.

After rebooting, both commands should return no matching kernel messages for a
successfully applied fix:

```bash
journalctl -b0 -k --grep=AE_AML_BUFFER_LIMIT
journalctl -b0 -k --grep='Marking method'
```

### Apple T2 Audio DSP

This is a fork of [Lemmyg's Apple-T2-Audio-DSP repo)](https://github.com/lemmyg/t2-apple-audio-dsp).
Leave him a proper GitHub star for his work. We adapted it to make
it work on Fedora. Most FIR files originate from lemmyg's
`t2-apple-audio-dsp`; the MacBook Pro 15,1 FIRs were generated from UMIK-1
measurements of that model by deqrocks.

This is PipeWire/WirePlumber DSP graphs and FIR files for Apple T2 audio.
The profiles in `firs/` are installed by `scripts/fedora/install-dsp.sh`.

Supported profiles:

- `MacBookPro16,1`
- `MacBookPro16,4`
- `MacBookAir9,1`
- `MacBookPro15,1`

The installer copies the matching files to:

```text
/usr/share/kait2en/audio-dsp/<profile>/
```

It generates a WirePlumber configuration at:

```text
/etc/wireplumber/wireplumber.conf.d/51-kait2en-t2-dsp.conf
```

The graph target is rewritten at install time to match the detected Apple T2
audio PCI device and KAIT2EN UCM sink/source names.

Required Fedora packages are installed by `install-dsp.sh`, not by the common
dependency installer:

- `pipewire`
- `pipewire-pulseaudio`
- `wireplumber`
- `pipewire-module-filter-chain-lv2`
- `lv2-bankstown`
- `lv2-triforce`
- `lsp-plugins-lv2`
- `lv2-swh-plugins`

### Suspend helper

The installer creates and enables `kait2en-suspend.service`. It runs
before suspend and after resume. The helper detects the local hardware and
only applies fixes that match the machine.

The suspend helper is installed as:

```text
/etc/systemd/system/kait2en-suspend.service # systemd service to run the .sh below
/usr/local/libexec/kait2en/kait2en-suspend.sh # actual script
```

The source files in this repository are:

```text
systemd/kait2en-suspend.service
scripts/fedora/kait2en-suspend.sh
scripts/fedora/install-suspend-service.sh
```

The service runs before `sleep.target` and again after resume. On the
MacBookPro15,1 it unloads `amdgpu` before suspend and loads it again after
resume. On BCM4377 systems it unloads `brcmfmac_wcc`, `brcmfmac`
and `hci_bcm4377` before suspend, then loads the modules again after
resume.

If the helper does not detect matching hardware, it does not unload anything.

### T2 CDC-NCM debug interface helper

The installer installs a udev rule that renames the internal Apple T2
CDC-NCM interface to `t2_ncm` and tells NetworkManager to ignore it. A separate
oneshot systemd service is started for that device and keeps it down. When you remove
this, NetworkManager will annoy you with notifications about failed connections from
`USB-Ethernet`, which actually is a debugging interface and not meant to be used for
intranet/internet connections.

As an alternative, you can disable `USB-Ethernet` auto connect in network settings.

The T2 CDC-NCM debug interface helper is installed as:

```text
/etc/systemd/system/kait2en-t2-ncm-down.service # systemd service triggered for the debug interface
/usr/local/libexec/kait2en/kait2en-t2-ncm-down.sh # actual script
/etc/udev/rules.d/90-kait2en-t2-network.rules # NetworkManager exclusion
```

The source files in this repository are:

```text
systemd/kait2en-t2-ncm-down.service
scripts/fedora/kait2en-t2-ncm-down.sh
scripts/fedora/install-t2-ncm-debug-service.sh
scripts/fedora/install-networkmanager-rules.sh
```

The udev rule detects the internal Apple T2 CDC-NCM interface by USB vendor and
product ID plus the `cdc_ncm` driver, renames it to `t2_ncm`, marks it
unmanaged for NetworkManager and asks systemd to start the helper service for
that device. The helper then forces `t2_ncm` back down for a short retry window
so late boot activity does not leave the debug interface up.

## T2-specific applications

The installer installs the required KAIT2EN apps:

- t2-fan-control # Adjustable fan curves with GUI
- t2-smc-control # Battery charge limit and SMC sensors in a GUI
- react-drm # Touchbar daemon - only on Touch Bar Macs

t2-fan-control and t2-smc-control can be found in the GNOME-App-Drawer
after installation is finished.

`t2-fan-control` installs:

- Binary: `/usr/local/bin/t2-fancontrol-gtk`
- Desktop file: `/usr/local/share/applications/org.t2fancontrol.gtk.desktop`
- Icon: `/usr/local/share/icons/hicolor/scalable/apps/org.t2fancontrol.gtk.svg`
- systemd service: `/usr/local/lib/systemd/system/t2-fancontrol.service`
- The service is enabled with `systemctl enable --now t2-fancontrol.service`

`t2-smc-control` installs:

- Binary: `/usr/local/bin/t2-smc-control`
- Desktop file: `/usr/local/share/applications/org.t2smccontrol.gtk.desktop`
- Icon: `/usr/local/share/icons/hicolor/scalable/apps/org.t2smccontrol.gtk.svg`
- systemd service: `/usr/local/lib/systemd/system/kait2en-t2-smc-charge-limit.service`
- Config file, after the first saved limit: `/etc/t2-smc-control/config.txt`
- The service is enabled with `systemctl enable kait2en-t2-smc-charge-limit.service`

When the GUI successfully sets the battery charge limit, it saves the selected
value in `/etc/t2-smc-control/config.txt`. On later boots,
`kait2en-t2-smc-charge-limit.service` restores that value through the `t2smc` hwmon
`battery_charge_limit` attribute.

`react-drm` installs:

- Source and build directory: `$HOME/react-drm` for the user who invoked `sudo`
- udev rule: `/etc/udev/rules.d/99-react-drm.rules`
- User service: `$HOME/.config/systemd/user/react-drm.service`
- The service is enabled with `systemctl --user enable --now react-drm.service`
- Service `WorkingDirectory`: `$HOME/react-drm/linux-touchbar-control-center`
- Service `ExecStart`: `node $HOME/react-drm/linux-touchbar-control-center/dist/index.js`

The installer builds `t2-fan-control` and `t2-smc-control` as the user who
invoked `sudo`, then installs them system-wide below `/usr/local`. `react-drm`
is copied into that user's home directory and built there.

## Kernel debug logs

KAIT2EN modules use `pr_debug()` for verbose driver messages. These messages
are hidden by default, even when you watch `dmesg` or `journalctl -k`.

If you want to see the verbose logging, you can either search and replace all
occurrencies of `pr_debug` in this repository with `pr_info` or use dynamic debug.

Fedora kernels support dynamic debug, so you can enable these messages at
runtime. Mount `debugfs` if it is not already mounted:

```bash
sudo mount -t debugfs none /sys/kernel/debug
```

Enable all debug messages from the `t2bce` module:

```bash
echo 'module t2bce +p' | sudo tee /sys/kernel/debug/dynamic_debug/control
```

Watch the kernel log:

```bash
sudo dmesg -w
```

or:

```bash
journalctl -k -f
```

For audio-only debug messages, enable just the audio source file:

```bash
echo 'file audio/audio.c +p' | sudo tee /sys/kernel/debug/dynamic_debug/control
```

Turn the messages off again:

```bash
echo 'module t2bce -p' | sudo tee /sys/kernel/debug/dynamic_debug/control
```

Next: [Revert T2 Linux Fedora to vanilla + KAIT2EN](../../migration/revert-t2linux-fedora.md)
