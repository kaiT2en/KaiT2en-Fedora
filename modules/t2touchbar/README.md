# t2touchbar

Touch Bar and keyboard-backlight driver fork for Intel Macs with a T2 chip.

## Changes

This tree carries the matching T2 Touch Bar and keyboard-backlight driver changes needed for the full suspend and resume fix set.

The installed module names are:

- `t2hid`
- `t2touchbar_bl`
- `t2touchbar_kbd`

## Butterfly keyboard debounce

Butterfly keyboards sometimes register one keystroke twice. `t2hid` can drop a press that follows the release of the same key within `debounce_ms` milliseconds. It is off by default.

Enable it on the kernel command line:

```
t2hid.debounce_ms=40
```

or at runtime:

```bash
echo 40 | sudo tee /sys/module/t2hid/parameters/debounce_ms
```

Dropped presses are logged with their interval once dynamic debug is enabled for `t2hid`.

## Required repositories

This fork is part of a set of fixes that make suspend/resume work on T2 Macs. Deploying the other fixes is mandatory to make it work.

- `apple-t2-bridge`
- `t2smc`

## Build and install

Before you proceed, make sure that the Linux T2 Headers are installed before continuing.

```bash
make
sudo make install
```

## Deploy

For Fedora-based distributions:

```bash
sudo dracut -f
```

For Debian-based distributions:

```bash
sudo update-initramfs -u
```

For Arch-based distributions:

```bash
sudo mkinitcpio -P
```
