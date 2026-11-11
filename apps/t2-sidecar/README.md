# t2-sidecar

Adds an extended display to GNOME for streaming the desktop to a tablet, the
counterpart of macOS Sidecar. The T2 encodes the stream through the
`t2-sidecar` encoder, see `website/docs/post-install/sidecar.md`.

```bash
t2-sidecar on              # mode used last, first time 1920x1200 at 60 Hz
t2-sidecar on 2560x1600@60
t2-sidecar status
t2-sidecar off
```

`t2-sidecar on` starts the user unit `t2-sidecar.service`. It runs
`t2-sidecar-display`, which asks Mutter for a virtual monitor through
`org.gnome.Mutter.ScreenCast.RecordVirtual` with `is-platform` and a list of
modes. GNOME then treats it like a connected display. The requested mode is
preferred, and 1920x1080, 1920x1200, 2560x1440, 2560x1600 and 2048x1536 can be
picked in GNOME Settings. The monitor exists while the unit runs. It ends with
`t2-sidecar off` or at logout.

Mutter gives every new virtual monitor a new serial number. A screen sharing
permission saved for one, like Sunshine's portal token, does not carry over to
the next.

`t2-sidecar on` then starts the user unit `t2-sidecar-sunshine.service`. It
runs only if `/usr/local/bin/sunshine` exists, the Sunshine build from the
documentation. It deletes Sunshine's portal token so GNOME asks for the new
monitor, and it is bound to `t2-sidecar.service`, so Sunshine stops together
with the virtual monitor.

## Files

| Source | Installed as |
| --- | --- |
| `t2-sidecar` | `/usr/local/bin/t2-sidecar` |
| `t2-sidecar-display` | `/usr/local/libexec/t2-sidecar-display` |
| `t2-sidecar.service` | `/usr/local/lib/systemd/user/t2-sidecar.service` |
| `t2-sidecar-sunshine.service` | `/usr/local/lib/systemd/user/t2-sidecar-sunshine.service` |

`t2-sidecar on MODE` writes the mode to `~/.config/t2-sidecar/env`. Without a
mode the file stays as it is. GNOME does not remember a mode for a virtual
monitor, since each one has a new serial, so `t2-sidecar-display` writes the
mode picked in GNOME Settings to the same file.

