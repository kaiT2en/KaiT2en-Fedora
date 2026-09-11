# t2sep

SEP mailbox transport for the Apple T2, PCI device `106b:1802`. It is the only
way to reach **AppleKeyStore** from Linux.

## Status: not shipped

Nothing in KAIT2EN consumes it, so it is not installed by
`install-dkms-modules.sh` (it is listed there for removal instead),
`AUTOINSTALL` is `no`, and the module carries no `MODULE_DEVICE_TABLE`. It
therefore never autoloads and never binds on a user machine. Load it by hand
for SEP work.

Touch ID does not go through here. The sensor hangs on the SEP endpoint `sbio`
inside bridgeOS and is reachable only over BridgeXPC on the BCE/NCM link.

## Use

```text
insmod t2sep.ko                  # map BAR4, report mailbox state via sysfs, send nothing
insmod t2sep.ko register_ool=1   # register OOL DMA, create /dev/t2sep
```

`/dev/t2sep` offers one ioctl, `T2SEP_IOC_EXCHANGE`: endpoint, operation,
request bytes in, response bytes out. The wire format lives in user space; see
`apps/t2-aks`.

Before registering either DMA buffer the driver sends Apple's side-effect-free
endpoint-0 NOP and requires its reply.

**SEP keeps the two registered 16 KiB DMA addresses and has no unregister
command.** After registration the module pins itself until reboot. Do not
force-unload or unbind it.

## Diagnostics

`mailbox_status`, `ool_status`, `capabilities` and `discovery` sysfs
attributes. Passive discovery decodes endpoint `0xfd` into a
FourCC-to-endpoint table and cannot be combined with OOL registration:

```text
insmod t2sep.ko discovery_window_ms=30000
insmod t2sep.ko start_sep=1 probe_testing=1
insmod t2sep.ko probe_control=1
```

`start_sep=1` performs the BAR4 writes from `AppleSEPIntelIOP::_startCPUGated()`
and is an active hardware operation, disabled by default.
