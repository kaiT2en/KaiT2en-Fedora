# T2 Touch Bar

Optional. The installer only asks on models with a Touch Bar. Without it,
Apple's native Touch Bar keeps working.

Keeps the Touch Bar dark until it is touched or Fn is pressed.

- Offers a media row, an F-key row and a row with print, insert, delete,
  home, end and page keys.
- Two-finger swipes change the volume on the dark bar.
- Two-finger swipes change the row on a lit bar.
- Three-finger swipes change the display brightness on a dark bar.
- Touch ID shows an "Unlock with Touch ID" prompt.
- Tracks from media players briefly appear on the dark bar. A touch on the
  title pauses.
- While something plays, a faint play triangle sits in the middle of the
  dark bar. A tap brings the track back, holding it pauses.
- After a pause from the bar, a tap on the pause bars plays on. They show
  along with the escape key.
- Remembers the last active row.
- Holding Fn switches between the media row and the last remembered row.
- Up to four personal keys can be configured in
  `~/.config/kait2en-touchbar/keys.toml`.

Reboot once after the first install.

Settings are in `/etc/kait2en/touchbar.toml`. To return to the native Touch
Bar, run `sudo systemctl --global disable kait2en-touchbar.service` and
reboot, or remove it with
`sudo /usr/local/src/KaiT2en-Fedora/apps/t2-touchbar/uninstall.sh`.
