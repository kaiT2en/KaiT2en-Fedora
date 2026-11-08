# Touch ID

KAIT2EN lets the finger you enrolled under macOS unlock the login screen and
confirm `sudo` on Linux. Nothing is enrolled on Linux. The Secure Enclave keeps
the fingerprint, compares it and sends Linux its verdict.

## Prerequisites

You need a finger enrolled under macOS, in System Settings > Touch ID.
The macOS installer tells you if none is enrolled while you are
still in macOS. Your macOS and Linux passwords do not have to match.

The Secure Enclave also needs a password to have unlocked its biometric keybag
once since the T2 itself last powered on. Rebooting the Mac does not power off
the T2, it only sleeps bridgeOS, so log into macOS once and it stays unlocked
across Linux reboots. A T2 panic or a power cut restarts the T2, and Touch ID
stays off until you log into macOS again with your password. If
`journalctl -u kait2en-t2-touchid` says no macOS user has a finger enrolled
although one is set up, this is why.

## Binding

Linux learns which enrolled finger the Secure Enclave saw, and `fprintd` remembers which of them belong to
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

## Turning it off

```bash
sudo authselect disable-feature with-fingerprint
sudo systemctl disable --now kait2en-t2-touchid
```
