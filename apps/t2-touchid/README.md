# t2-touchid

Bridges the Apple T2 Touch ID sensor to `fprintd`, so that login and `sudo`
accept the finger enrolled under macOS.

`fprintd` cannot talk to the T2 itself: its unit sets
`RestrictAddressFamilies=AF_UNIX AF_LOCAL AF_NETLINK`, and the sensor is only
reachable over IPv6 on the CDC-NCM link. This daemon does that part and feeds
`fprintd` through libfprint's virtual storage device, so stock `fprintd`, stock
`libfprint` and stock `pam_fprintd` are used, with nothing patched.

## Arming

The virtual device listens on a Unix socket only while `fprintd` holds it open,
which is exactly the duration of one authentication attempt. A successful
`connect()` is therefore the signal to put the sensor into match mode, and the
daemon does nothing at any other time.

This is not a detail. libfprint queues a scan that arrives while no operation is
pending, and the next verify consumes it immediately. A daemon that free-ran
would let a finger touched at an idle moment satisfy a `sudo` minutes later.

Whoever can write to the socket authenticates, so it lives in a root-only
`RuntimeDirectory` with mode 0700.

## Binding

Enroll the finger under macOS; this daemon never enrolls. What it does is bind:
`fprintd` needs a record saying that a given SEP identity belongs to a given
Linux account, and the daemon makes that record itself. With `--bind-user` set
(the installer sets it to the account that ran it) it checks on every start
which enrolled identities `fprintd` does not know, runs `fprintd-enroll` for
them and answers the scan with the identity straight away. No touch is needed
for that, because it is only a statement of ownership; the finger is still
required at every login.

Without `--bind-user`, or for another account, a manual `fprintd-enroll` with
a touch does the same, and one touch is enough because the daemon sets the
enrolment to a single stage.

`fprintd-enroll` labels the finger `right-index-finger` unless told otherwise.
The label is only a name: the T2 decides which finger matched, and the daemon
reports its identity. Use the finger you actually enrolled under macOS, and
name it accordingly:

```text
fprintd-enroll -f left-thumb
```

The accepted names are `left-thumb`, `right-thumb` and
`{left,right}-{index,middle,ring,little}-finger`.

Repeat that per finger if several are enrolled under macOS; each has its own
identity and gets its own binding.

The daemon finds the macOS user whose fingers are enrolled by itself; the SEP
answers with an empty inventory for a user id that has none. Set
`T2_TOUCHID_UID` in `/etc/kait2en/t2-touchid.conf` only if several macOS users
have Touch ID, and then to the **macOS** id (501 and up), not the Linux one.

Nothing else is needed from macOS. The keybag and catacomb path is a separate
matter: it releases keys, and a login only needs the SEP's verdict.
