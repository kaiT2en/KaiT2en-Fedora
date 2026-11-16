# Configuring modifier key positions

The position of some modifier keys on Apple keyboards, such as the Control key, might be impractical for some users,
and alternative positions might make keyboard shortcuts more familiar.
`t2hid` exposes options to swap some specific keys.

## Swapping Control and Command keys

This will switch the Control and the Command keys. All Command keys will act as Control keys and vice versa.

Try it at runtime first:

```bash
echo 1 | sudo tee /sys/module/t2hid/parameters/swap_ctrl_cmd
```

0 turns it off again.

To keep it across reboots, add the kernel argument:

```bash
sudo grubby --update-kernel=ALL --args="t2hid.swap_ctrl_cmd=1"
```

Reboot for the change to take effect.

## Swapping Control and Fn keys

This will switch the Fn and left Control keys.

Try it at runtime first:

```bash
echo 1 | sudo tee /sys/module/t2hid/parameters/swap_fn_leftctrl
```

0 turns it off again.

To keep it across reboots, add the kernel argument:

```bash
sudo grubby --update-kernel=ALL --args="t2hid.swap_fn_leftctrl=1"
```

Reboot for the change to take effect.

## Swapping Option and Command keys

This will switch the Option and the Command keys. All Command keys will act as Option keys and vice versa.

Try it at runtime first:

```bash
echo 1 | sudo tee /sys/module/t2hid/parameters/swap_opt_cmd
```

0 turns it off again, while 2 swaps only the left Option and left Command keys.

To keep it across reboots, add the kernel argument:

```bash
sudo grubby --update-kernel=ALL --args="t2hid.swap_opt_cmd=1"
```

Reboot for the change to take effect.

## Swapping keys at the desktop level

For GNOME users, it's possible to perform more flexible remapping using GNOME Tweaks, but this will apply to all keyboards, not just Apple ones.
This is an alternative to the `t2hid` configuration above, and might result in swapping the keys back if the same configuration is applied in both GNOME Tweaks and in `t2hid`.

Install GNOME Tweaks (either from the Software app or with `sudo dnf install gnome-tweaks`) and open it.

From the sidebar, navigate to Keyboard, then click on Additional Layout Options. In this window, particularly inside the "Ctrl position" section, you can check multiple options as you see fit.

Keep in mind that, in this context, "Win" refers to Command keys and "Alt" refers to Option keys.
