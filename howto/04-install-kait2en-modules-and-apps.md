# Install KaiT2en modules and apps

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md) (you are here)

Previous: [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md) | [Back to README](../README.md)

Run this command from the KaiT2en repository root on your Fedora system:

```bash
sudo bash ./scripts/fedora/install.sh
```

This runs all required installation steps in order:

- Fedora dependencies
- KaiT2en kernel arguments
- KaiT2en DKMS modules
- NetworkManager exclusion for the internal T2 CDC-NCM debug interface
- systemd helper to keep the internal T2 CDC-NCM debug interface down
- initramfs rebuild
- KaiT2en suspend helper service
- KaiT2en apps
- react-drm, last and only on Touch Bar Macs

This will take a few minutes.
Stay around to enter needed confirmations while the script is running.
Please reboot after the script completed without errors.

```bash
sudo reboot
```

The installer also installs the required KaiT2en apps:

- t2-fan-control # Adjustable fan curves with GUI
- t2-smc-control # Battery charge limit and SMC sensors in a GUI
- react-drm # Touchbar daemon, last and only on Touch Bar Macs

t2-fan-control and t2-smc-control can be found in the GNOME-App-Drawer
after installation is finished.

The installer also enables `kait2en-suspend.service`. It runs before suspend and
after resume. The helper detects the local hardware and only applies fixes that
match the machine.

The installer also installs a udev rule that renames the internal Apple T2
CDC-NCM interface to `t2_ncm` and tells NetworkManager to ignore it. A separate
oneshot systemd service is started for that device and keeps it down because
KaiT2en uses it only for debugging, not for normal networking.

## Suspend helper

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

The service runs before `sleep.target` and again after resume. On affected
dual-GPU MacBook Pro models it unloads `amdgpu` before suspend and loads it
again after resume. On BCM4377 systems it unloads `brcmfmac_wcc`, `brcmfmac`
and `hci_bcm4377` before suspend, then loads only the modules it unloaded after
resume.

If the helper does not detect matching hardware, it does not unload anything.

## T2 CDC-NCM debug interface helper

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
