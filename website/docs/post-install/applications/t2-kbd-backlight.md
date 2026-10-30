# T2 Keyboard Backlight

Installed on every T2 MacBook, with or without a Touch Bar.

Turns the keyboard backlight off after 30 seconds without input and back on
with the next key press, trackpad touch or Touch Bar touch.

The setting lives in `/etc/kait2en/kbd-backlight.toml`:

```toml
enabled = true
idle_timeout_ms = 30000
```

`idle_timeout_ms` accepts 5,000 to 3,600,000 ms. `enabled = false` leaves the
backlight alone. Apply changes with:

```bash
sudo systemctl restart t2-kbd-backlight
```

Updates never overwrite this file.
