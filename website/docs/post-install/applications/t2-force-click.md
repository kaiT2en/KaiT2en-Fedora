# T2 Force Click

<img class="app-shot" src="img/apps/t2-force-click.webp" alt="T2 Force Click with click force, Force Click force and action">

Sets the pressure for a normal click and for Force Click, the harder press
after it. Tap-to-click, scrolling and gestures stay with libinput.

- **Click force**: Light, Medium or Firm.
- **Force click force**: relative to the click force.
- **Disable Force Click** keeps the normal click only.

**On Force Click** binds one action:

- **Copy / Paste**: the first Force Click sends Ctrl+C, the next Ctrl+V, and
  so on.
- **Custom key combo**: a combination set with **Record shortcut…**, sent
  through a virtual keyboard. Works under Wayland and X11.
- **Run command**: a shell command, run as the logged-in user.

Force Click is also the key event `BTN_TASK` on the trackpad's input device,
so desktop shortcuts and other tools can bind it directly. A physical
three-finger click is a middle click.

The service `t2-force-click.service` applies the settings at boot and runs the
action. Settings are in `/etc/t2-force-click/config.txt`.
