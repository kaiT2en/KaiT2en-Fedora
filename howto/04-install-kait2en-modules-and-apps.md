# Install KaiT2en modules and apps

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md) (you are here)

Previous: [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md) | [Back to README](../README.md)

This step will install DKMS modules, scripts and apps.

Run this command from the KaiT2en repository root on your Fedora system:

```bash
sudo bash ./scripts/fedora/install.sh
```

This runs all required installation steps in order:

It will take a few minutes to compile the modules and apps against the kernel.
Stay around to enter needed confirmations while the script is running.
Please reboot after the script completed without errors.

```bash
sudo reboot
```

## Apps

The installer installs the required KaiT2en apps:

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

## Suspend helper

The installer creates and enables `kait2en-suspend.service`. It runs before suspend and
after resume. The helper detects the local hardware and only applies fixes that
match the machine.

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
and `hci_bcm4377` before suspend, then loads the modules again after
resume.

If the helper does not detect matching hardware, it does not unload anything.

## T2 CDC-NCM debug interface helper

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
