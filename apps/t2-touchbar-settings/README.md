# Touch Bar Settings

GTK4/libadwaita app for the app rows of [kait2en-touchbar](../t2-touchbar):
give single apps a Touch Bar row of their own keys, like BetterTouchTool does
on macOS. It edits `$XDG_CONFIG_HOME/kait2en-touchbar/apps.toml` and restarts
the Touch Bar service on save. Nothing runs as root.

## Features

- Add an app by switching to it (the window class is read from the focused
  window after a short countdown) or by picking it from the installed apps
- Per key: a label of up to 12 characters and either a shortcut or a command
- Record a shortcut by pressing it. Keys are stored by their US layout
  position, the way the daemon sends them
- Reorder and remove keys, with a preview of the row; keys past the eleventh
  stay off the bar and are marked
- Entries the daemon would skip are marked and block saving
- Saving keeps the previous file as `apps.toml.bak`

Following the focused app needs the
[Focused Window D-Bus](https://extensions.gnome.org/extension/5592/focused-window-d-bus/)
GNOME Shell extension. The app shows a banner when it does not answer.

## Build and install

```bash
make build
make test
make install-user        # ~/.local, no root
sudo make install        # or /usr/local
```
