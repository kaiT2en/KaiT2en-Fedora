# t2-aks

User-space side of the Apple T2 **AppleKeyStore** transport. It builds the AKS
wire format, including the SHA-256 header digest, and exchanges it with the
Secure Enclave through the `t2sep` ioctl. The kernel keeps the bytes opaque;
the format lives here.

## Status: not installed

`install-apps.sh` does not install it, and `t2sep` is not shipped either. This
is a research tool: build and run it from the repository.

```text
make
insmod ../../modules/t2sep/t2sep.ko register_ool=1
sudo target/release/t2-aks capabilities
```

## Commands

| Command | AKS op | Effect |
| --- | --- | --- |
| `capabilities` | `0x4d` | read the capability word |
| `load-keybag FILE` | `0x03` | load a keybag, print its handle |
| `copy-uuid HANDLE` | `0x06` | read a loaded keybag's UUID prefix |
| `unlock HANDLE` | `0x04` | unlock a keybag; password on stdin |
| `set-system HANDLE SPECIAL` | `0x0d` | bind the keybag as the system keybag |

**`set-system` deletes the Touch ID enrollment.** It was verified on hardware:
after running it the SEP reported no enrolled finger. Do not use it. Unlocking
the system bag (`-502`) has crashed the SEP with a CATERR and host reset.

Unlock attempts are rate limited by SEP. Do not guess the password.

Touch ID does not need any of this: the keybag path and the biometric path are
separate.
