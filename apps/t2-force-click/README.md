# T2 Force Click

GTK4/libadwaita app and daemon for the `t2_precision_trackpad` /
`t2_trackpad_actuator` kernel modules: configure click and force click
pressure, and bind an action to a force click.

The required kernel drivers are in this repository under
`modules/t2_precision_trackpad` and `modules/t2_trackpad_actuator`.

## Features

- Slider for click pressure (`click_strength`: light/medium/firm)
- Slider for force click pressure, relative to the click threshold
  (`force_click_threshold_percent`)
- Checkbox to turn force click off entirely (`force_click_enabled`) while
  keeping the plain click working, for desktops with tap-to-click disabled
- Force click is exposed as a `BTN_TASK` key event on the trackpad's input
  device for any third-party tool (DE shortcuts, `xbindkeys`, `evtest`,
  `libinput debug-events`) to bind independently of this app
- One optional action per force click: alternating copy/paste; a recorded key
  combo (synthesized through a virtual keyboard, works under
  X11 and Wayland); or an arbitrary shell command. Commands run as the active
  desktop user, not as root.
- A physical three-finger click is already a middle click directly from the
  trackpad; it does not need a Force Click binding.
- A persistent root daemon (`t2-force-click --daemon`) applies the saved
  thresholds at boot and runs the configured action; the GUI is a thin client
  talking to it over `/run/t2-force-click/daemon.sock`. The daemon verifies
  the local peer identity and accepts configuration only from the active user
  session (or root).

## Requirements

- A T2 Mac with the internal trackpad
- `t2_precision_trackpad` and `t2_trackpad_actuator` kernel modules loaded
- Rust/Cargo
- GTK 4 and libadwaita development packages

## Build

```sh
make build
```

## Install

```sh
sudo make install
```

This installs:

- `/usr/local/bin/t2-force-click`
- `/usr/local/share/applications/org.t2forceclick.gtk.desktop`
- `/usr/local/share/icons/hicolor/scalable/apps/org.t2forceclick.gtk.svg`
- `/usr/local/lib/systemd/system/t2-force-click.service` (enabled and
  started automatically)

## Installed files and state

```text
/usr/local/bin/t2-force-click
/usr/local/share/applications/org.t2forceclick.gtk.desktop
/usr/local/share/icons/hicolor/scalable/apps/org.t2forceclick.gtk.svg
/usr/local/lib/systemd/system/t2-force-click.service
/etc/t2-force-click/config.txt
/run/t2-force-click/daemon.sock
```

The socket is runtime state and disappears on shutdown. The driver settings
the daemon owns are exposed while the trackpad driver is loaded at:

```text
/sys/module/t2_precision_trackpad/parameters/click_strength
/sys/module/t2_precision_trackpad/parameters/force_click_threshold_percent
/sys/module/t2_precision_trackpad/parameters/force_click_enabled
```
