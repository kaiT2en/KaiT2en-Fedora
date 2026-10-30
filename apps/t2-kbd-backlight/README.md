# T2 keyboard backlight

Turns the keyboard backlight off after a while without input and back on with
the next input, on every T2 MacBook, with or without a Touch Bar.

Key presses, trackpad movement and Touch Bar touches count as input. A finger
resting on the trackpad counts before it moves. The backlight comes back at
the level it had before, fading in over 500 ms. It fades out over 700 ms.

The daemon runs as a system service, so it also works on the login screen.
The level is set through UPower, which keeps the GNOME brightness slider in
sync. A level chosen while the backlight is off is kept as the new level.

Before suspend the daemon puts the backlight back on at its previous level.
The keyboard driver restores the last level when the keyboard comes back after
resume, so the lock screen comes back lit instead of dark.

## Configuration

`/etc/kait2en/kbd-backlight.toml`:

```toml
enabled = true
idle_timeout_ms = 30000
```

`idle_timeout_ms` accepts 5,000 to 3,600,000 ms. With `enabled = false` the
daemon exits at start and leaves the backlight alone. Apply changes with:

```bash
sudo systemctl restart t2-kbd-backlight
```

Updates never overwrite this file.

## Installation

The KaiT2en installer sets it up on every T2 MacBook. On its own:

```bash
make build
sudo make install
```

`sudo make uninstall` removes it again and keeps the configuration.
