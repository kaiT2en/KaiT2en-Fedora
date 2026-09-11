# Touch ID

KAIT2EN lets the finger you enrolled under macOS unlock the login screen and
confirm `sudo` on Linux. Nothing is enrolled on Linux: the Secure Enclave keeps
the fingerprint, compares it, and Linux only receives its verdict.

## Prerequisites

You need a finger enrolled under macOS, in System Settings > Touch ID. 
The macOS installer tells you if none is enrolled while you are
still in macOS. Your macOS and Linux passwords do not have to match.

## Fresh installation

The regular installer sets everything up, and the fingers enrolled under macOS
are bound to the account that ran it as soon as the sensor is reachable. There
is nothing to do. From then on the login screen and `sudo` offer the finger;
the password keeps working as before.

## Existing installation

Update KAIT2EN, which runs the installer again:

```bash
kait2en-install
```

## The binding

The Secure Enclave keeps the fingerprint and compares it. Linux only learns
which enrolled finger it saw, and `fprintd` remembers which of them belong to
your account. That binding is made without touching the sensor, because it is
only the statement that the macOS-enrolled fingers are yours, the same
assumption macOS itself makes. The finger still has to be on the sensor at
every login.

`fprintd-list $USER` shows the bound fingers. The sensor does not say which
finger a template is, so the labels are positional: the first is
`right-index-finger`, the second `right-middle-finger`, and so on. Fingers
enrolled under macOS later are bound the next time the bridge starts, for
example after a reboot.

## Several users

The bridge looks for the macOS user that has fingers enrolled by itself and
binds them to the Linux account that ran the installer. If that is not the
right pairing, edit `/etc/kait2en/t2-touchid.conf`: `T2_TOUCHID_UID` is the
macOS id (`id -u` under macOS, usually 501 or 502) and `T2_TOUCHID_BIND_USER`
the Linux account. Restart `kait2en-t2-touchid` afterwards. Any other Linux
user can bind a finger by hand with `fprintd-enroll` and a touch.

## What to expect

- A finger that is not recognised is simply asked for again.
- If the sensor cannot be reached, or the bridge is not running, there is no
  fingerprint prompt at all and the password is asked for right away. A broken
  bridge never makes you wait.
- The prompts say *swipe* and the device is listed as *Virtual device with
  storage and identification for debugging*. Both come from the libfprint
  driver KAIT2EN drives and change only once a native driver exists upstream.
  Press the finger flat on the sensor; do not swipe.

## Turning it off

```bash
sudo authselect disable-feature with-fingerprint
sudo systemctl disable --now kait2en-t2-touchid
```
