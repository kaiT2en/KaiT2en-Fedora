# react-drm

react-drm provides a React renderer for drawing directly to Linux DRM/KMS
displays using libdrm and Cairo. This repository includes a control center
that replaces the standard Touch Bar interface on T2 MacBooks running Linux.
This copy is integrated with KaiT2en and is installed through the KaiT2en
application installer.

The control center provides:

- Function keys and an optional on-screen Escape key
- Media controls, volume and display brightness
- Application-aware controls for browsers, media players and file managers
- CPU, memory, temperature, network and battery information
- Audio visualization, a focus timer and small games
- Automatic detach and re-attach during suspend and resume

## Installation

react-drm replaces the existing Touch Bar interface. `tiny-dfr`,
`mac-touchbar-plus` and other Touch Bar daemons must not run alongside it.

From the KaiT2en repository root, install or update only react-drm with:

```sh
sudo ./scripts/fedora/install-apps.sh --react-drm-only
```

The react-drm directory provides an equivalent shortcut that is run as the
desktop user:

```sh
./apps/react-drm/install.sh
```

The installer:

- verifies that the Mac model has a T2 Touch Bar;
- installs the Fedora build and runtime dependencies;
- removes conflicting Touch Bar daemons;
- installs the udev rules and required user groups;
- copies the current source to `~/react-drm` and builds it there;
- installs Window Monitor Pro when GNOME is active;
- installs and starts `react-drm.service` for the invoking user.

Run the same command after updating the KaiT2en repository. It rebuilds only
react-drm; `t2-fan-control` and `t2-smc-control` are not rebuilt. The complete
KaiT2en application installer remains available as:

```sh
sudo ./scripts/fedora/install-apps.sh
```

### Uninstall

Run the separate uninstaller as the desktop user:

```sh
./apps/react-drm/uninstall.sh
```

It stops and removes the react-drm user service, restores the firmware Touch
Bar interface and removes the react-drm udev rules. Project files, npm
dependencies, system packages and `video`/`input` group memberships are left
unchanged.

### Service status

Check its status and log with:

```sh
systemctl --user status react-drm.service
journalctl --user -u react-drm.service -b
```

The service runs without root privileges. It attaches the Touch Bar when the
graphical session starts, restores the firmware interface when the session
ends and handles suspend and resume. The firmware function-key strip remains
available before login and after logout.

## Manual start

Stop the user service before running the control center manually:

```sh
systemctl --user stop react-drm.service
cd apps/react-drm/linux-touchbar-control-center
npm run dev
```

`npm run dev` is the development entrypoint and keeps hot reload enabled. The
installed systemd service uses the compiled production build instead.

## Active window integration

Application-specific controls require an active-window backend. The KaiT2en
installer deploys the required backend and react-drm selects it automatically:

- GNOME Wayland uses
  [Window Monitor Pro](https://extensions.gnome.org/extension/8549/window-monitor-pro/),
  maintained by the react-drm developer
- KDE Plasma Wayland uses
  **TouchBar Dynamic Shortcuts**, a KWin script installed and enabled by
  the KaiT2en installer
- Hyprland uses its IPC socket
- Xorg uses `xprop`

On GNOME Wayland the KaiT2en installer includes and enables Window Monitor Pro.
A logout and login may be required when the extension is installed for the
first time. `xprop` must be installed for Xorg sessions. Unsupported Wayland
desktops can still run the Touch Bar UI, but application-specific controls
that depend on the focused window will not work.

## Media progress bar support (mpris)

The control center displays a visual playback progress bar for media players
that expose an MPRIS2 D-Bus interface. Spotify registers its own
`org.mpris.MediaPlayer2.spotify` service and works without additional setup.

Current Brave and Chromium builds expose their media sessions directly through
MPRIS2. This also works when the browser is installed as a Flatpak. Verify the
active service during playback with:

```sh
busctl --user list | grep org.mpris.MediaPlayer2
```

react-drm recognizes `brave` and `chromium` services directly. Some other
Chromium-based browsers do not expose MPRIS2. For those browsers, Plasma Browser
Integration can provide an
`org.mpris.MediaPlayer2.plasma-browser-integration` service:

- [Chrome Web Store](https://chromewebstore.google.com/detail/plasma-integration/cimiefiiaegbelhefglklhhakcgmhkai)
- [Firefox Add-ons](https://addons.mozilla.org/en-US/firefox/addon/plasma-integration/)

The extension requires the native Plasma Browser Integration host supplied by
the distribution; the extension alone cannot publish a D-Bus service. The
progress bar works on any desktop once an MPRIS2 service is present. It updates
live, shows album art embedded in the track title row, and supports seek
(tap/drag on the progress track or use the skip-back/skip-forward buttons).

## Keyboard shortcuts

Physical keyboard shortcuts recognised by the control center. All shortcuts
are injected via uinput. They work regardless of which application has focus.

### Layer navigation

| Shortcut | Action |
|---|---|
| Long-press **Fn** | Toggle the F‑key layer (F1–F12 and Esc on wide Touch Bars). Hold again to return. |
| Long-press **Right Alt** (⌥) | Toggle the app dock. Long-press again to close it and return to the previous layer. |

### Screenshots

| Shortcut | Action |
|---|---|
| **Ctrl + Alt + S** | Save the current Touch Bar screen as a PNG into `~/Pictures/touchbar/`. |

### Browser shortcuts

Available when a supported browser window is focused and the Browser Panel is
shown on the left side of the split layer.

| Shortcut | Action |
|---|---|
| **Alt + ←** | Back |
| **Alt + →** | Forward |
| **Ctrl + R** | Reload |
| **Alt + Home** | Home |
| **Ctrl + T** | New tab |
| **Ctrl + W** | Close tab |
| **Ctrl + Tab** | Next tab |
| **Ctrl + Shift + Tab** | Previous tab |

Key overrides per browser can be configured in `linux-touchbar-control-center/config.ts`
(`BROWSER_KEY_OVERRIDES`).

## Konsole integration

The Konsole panel can show suggestions without additional configuration.
Sending commands requires Konsole's security-sensitive D-Bus API:

```sh
kwriteconfig6 --file konsolerc --group KonsoleWindow --key EnableSecuritySensitiveDBusAPI true
```

The key must be stored in the `[KonsoleWindow]` group of
`~/.config/konsolerc`. Konsole reads it only at startup, so close all Konsole
windows before starting it again. With `UseSingleInstance=true`, the process
continues running while any window remains open.

Command suggestions use read-only D-Bus methods and work without this setting.
Enabling the security-sensitive API allows any process on the session bus to
send text and commands to open Konsole sessions.
