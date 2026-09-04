# T2 SMC Control

Small GTK/libadwaita control app for T2 Macs using the `t2smc` kernel driver.

The required kernel driver is maintained here:

https://github.com/deqrocks/t2-smc

## Features

- Shows fan speeds reported by `t2smc`
- Shows temperature sensors exposed through hwmon
- Shows all dynamically discovered `P...` SMC power stats below the primary
  battery/adapter values, using known human-readable key descriptions
- Shows event-driven SMC battery and power-adapter telemetry
- Shows the `t2smc` RTC (in UTC) when available and can write the current
  system time to it
- Shows the battery charge limit read-only

## Requirements

- A T2 Mac
- Linux with the `t2smc` kernel module loaded
- Rust/Cargo
- GTK 4 and libadwaita development packages
- `pkexec` and `hwclock` for writing the hardware clock

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

## Battery charge limit

The charge limit is shown here. `t2smc` exposes it as
`/sys/class/power_supply/BAT0/charge_control_end_threshold`, so the desktop
environment drives it natively and restores it on its own at every start.
GNOME puts it under Settings -> Power -> Battery Charging, KDE under Energy
Saving -> Charge Limit.

The app auto-detects `t2smc`/`macsmc` under `/sys/class/hwmon`.
