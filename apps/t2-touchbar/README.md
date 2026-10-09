# T2 Touch Bar

T2 Touch Bar replaces Apple's built-in Touch Bar row with its own
dark-first display. The bar stays black until you touch it or press Fn,
learns how long to stay lit, offers a media row and an F-key row (hold Fn
to switch), shows a Touch ID prompt and gives haptic
feedback on key presses. It saves power because the bar is off most of
the time.

Three fixed layers are available:

- media keys
- F-keys
- keys the MacBook keyboard lacks: print, insert, delete, home, end, page
  up and page down

A two-finger swipe on the lit bar moves through the layers in a ring (left
brings in the next one from the right, sliding and fading). A 600 ms Fn hold
switches between media keys and the other layer used last. The last layer is
remembered.

Up to four personal keys can fill the free space on the third layer. They live in
`$XDG_CONFIG_HOME/kait2en-touchbar/keys.toml`, which updates and uninstalls 
never touch. The daemon creates it with commented examples on its
first start. Each key has a text label and either sends a key
combination or starts a command:

```toml
[[key]]
label = "~"
send = "altgr+]"    # types ~ on the German layout

[[key]]
label = "|"
send = "altgr+iso"

[[key]]
label = "term"
run = "kgx"         # started through systemd-run --user
```

Modifiers are `ctrl`, `shift`, `alt` (or `option`), `altgr` and `super`. Key
names follow the US key positions: letters, digits, `f1`–`f12`, punctuation
such as `-`, `[`, `` ` `` or `\`, `iso` for the extra key next to the left
shift, and names like `space`, `tab`, `enter`, `left` or `delete`. Restart the
service after editing: `systemctl --user restart kait2en-touchbar`.

A two-finger swipe on the dark bar changes the volume without waking it: right
is louder, left is quieter. A three-finger swipe there changes the
display brightness the same way. The esc slot works on the dark bar too and
sends Esc without waking it. A short Fn press and a touch elsewhere both wake
the saved layer. The initial five-second illumination timeout learns
up to a hard thirty-second ceiling when a user repeatedly has to wake the bar
again. It decays slowly when the learned extension is unused.

While `t2-touchid` reports an authentication, all keys disappear and the bar
shows "Unlock with Touch ID" with an arrow that keeps nudging towards the
sensor. The prompt is drawn by this program. No Apple
artwork is included. This works for sudo and the GNOME lock screen in the
running user session.

Holding previous or next seeks in the active media player (MPRIS) instead of
skipping the track. A tap still skips.

When a media player starts a new track while the bar is dark, "Artist – Title"
fades in for five seconds with previous/next keys at both ends, so an unwanted
track can be skipped without waking the bar. `show_track_changes = false`
turns this off.

Accepted key presses use `t2_trackpad_actuator` to give a bit of haptic feedback.

The panel itself only has two brightness steps. The key glyphs are therefore
dimmed in software to follow the keyboard backlight, so the bar and the keys
look alike. A readable minimum remains while the keyboard backlight is off.

Keyboard, trackpad and Touch Bar activity show the escape key and restore the
keyboard backlight to its most recently selected level. After 30 seconds
without activity, the daemon reads and remembers the current user level, then
turns off both backlights. Input is handled through libinput events. The daemon
does not poll while idle. The escape key and keyboard backlight fade over 700 ms.
Set `activity_backlight = false` to turn this
behavior off, or change `activity_timeout_ms` (5,000–300,000 ms) to adjust the
idle period.

## Installation

For a standalone Fedora installation from the repository, run:

```sh
sudo ./apps/t2-touchbar/install.sh
```

The script installs its build dependencies, configures device access and the
user service, disables conflicting Touch Bar daemons, and removes the Cargo
build directory after copying the release binary.

The Fedora installer creates the `kait2en-touchbar` device-access group, adds
the invoking user, installs the global user unit, and replaces conflicting
Touch Bar daemons. A reboot is required after the first
group assignment.

To go back to Apple's native Touch Bar without uninstalling, disable the
daemon and reboot:

```sh
sudo systemctl --global disable kait2en-touchbar.service
```

The attach service then leaves the firmware row in place at boot. Enable it
again with `sudo systemctl --global enable kait2en-touchbar.service`.

State is stored at `$XDG_STATE_HOME/kait2en-touchbar/state.toml`. Inspect or
reset it with:

```sh
kait2en-touchbar --status
kait2en-touchbar --reset-learning
```

The installed defaults live in `/etc/kait2en/touchbar.toml`. A personal
`$XDG_CONFIG_HOME/kait2en-touchbar/config.toml` replaces them when present.
The learned values remain separate from configuration so editing the bounds
does not destroy the history. Values outside new bounds are clamped on load.

Keys are drawn anti-aliased in Adwaita Sans (an Inter derivative close to the
San Francisco legends on the keyboard) with round-capped icons.

The daemon is event-driven. With the bar dark it blocks on input and D-Bus
file descriptors. There is no periodic inference loop. "Dark" means a black
frame plus backlight level zero. The separate `05ac:8102` brightness controller
is then allowed to runtime-suspend. The `05ac:8302` display/touch device remains
awake, because suspending it would also remove touch-to-wake.

The DRM setup and input architecture are derived from tiny-dfr under its MIT
license. The KAIT2EN implementation is GPL-3.0-or-later. The upstream notice
is in `THIRD-PARTY-NOTICES.md`.

Touch ID is a trademark of Apple Inc. This project is not affiliated with Apple.
