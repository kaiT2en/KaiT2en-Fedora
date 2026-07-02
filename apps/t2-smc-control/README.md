# T2 SMC Control

Small GTK/libadwaita control app for T2 Macs using the `t2smc` kernel driver.

The required kernel driver is maintained here:

https://github.com/deqrocks/t2-smc

## Features

- Shows fan speeds reported by `t2smc`
- Shows temperature sensors exposed through hwmon
- Shows the `t2smc` RTC when available
- Reads and writes the battery charge limit through `battery_charge_limit`
- Saves the selected charge limit and restores it at boot through systemd

## Requirements

- A T2 Mac
- Linux with the `t2smc` kernel module loaded
- Rust/Cargo
- GTK 4 and libadwaita development packages
- `pkexec` for applying the charge limit from the GUI

## Build

```sh
make build
```

## Install

```sh
sudo make install
```

This installs:

- `/usr/local/bin/t2-smc-control`
- `/usr/local/share/applications/org.t2smccontrol.gtk.desktop`
- `/usr/local/share/icons/hicolor/scalable/apps/org.t2smccontrol.gtk.svg`
- `/usr/local/lib/systemd/system/kait2en-t2-smc-charge-limit.service`

The systemd service is enabled during install. It applies the saved charge limit
on boot when `/etc/t2-smc-control/config.txt` exists.

## Persistent charge limit

When the GUI successfully sets a battery charge limit, it stores the selected
value in:

```text
/etc/t2-smc-control/config.txt
```

The file format is:

```text
charge_limit=80
```

The installed binary also supports headless root commands:

```sh
sudo t2-smc-control --set-charge-limit 80
sudo t2-smc-control --apply-saved-charge-limit
```

The app auto-detects `t2smc`/`macsmc` under `/sys/class/hwmon`. Battery charge limit support depends on the loaded driver and the available SMC keys on the machine.
