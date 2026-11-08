# Automatic brightness

Optional. The installer only asks on MacBook Pro and MacBook Air models with
GNOME. Without it, GNOME keeps its own brightness handling.

Adjusts the display brightness to the ambient light the way macOS does.

- Changes fade in slowly over a few seconds, in steps too small to see.
- Changes in light of less than about 20 % are ignored, so the screen does
  not follow every shadow.
- Setting the brightness by hand pauses it. One second later it learns the
  new brightness for the current light and keeps following from there.
- The slider and the brightness keys always start from the real brightness.

While it runs, GNOME's own automatic brightness and the dimming of the screen
when idle are switched off. Both come back when it is removed. A small GNOME
Shell extension, KaiT2en Auto Brightness, keeps the slider in sync and is
switched on by the service.

The learned curve is in `~/.local/state/t2-autobrightness/curve`. Deleting
it while the service is stopped starts again from the factory curve of the
model. To see what it does, run
`journalctl --user -u kait2en-t2-autobrightness -f`. Remove it in KaiT2en
Update or with
`sudo /usr/local/src/KaiT2en-Fedora/scripts/fedora/install-autobrightness.sh --remove`.
