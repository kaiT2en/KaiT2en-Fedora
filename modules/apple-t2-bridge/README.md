# apple-t2-bridge

Drivers for the T2 chip of Intel Macs, laid out like the kernel tree so that
the sources map directly onto upstream patches:

| Path | Module | Function |
|---|---|---|
| `drivers/platform/x86/apple/` | `apple-t2-bridge` | Bridge Controller (PCI 106b:1801), DMA queues to the T2 |
| `drivers/usb/host/apple-t2-vhci*` | `apple-t2-vhci-hcd` | virtual USB host controller for keyboard, trackpad, Touch Bar |
| `sound/pci/apple-t2/` | `snd-apple-t2` | audio (PCI 106b:1803) |
| `include/linux/apple-t2-bridge.h` | | client interface of the bridge |

The bridge creates the auxiliary devices `apple_t2_bridge.vhci` and
`apple_t2_bridge.ave`. The AVE and Sidecar drivers live in
`t2-services/t2-ave/kernel/` and are built in the same DKMS package.

The ALSA UCM profile for the audio driver is in `modules/apple-t2-alsa-ucm-conf`.

## Debug

```
apple_t2_bridge.dyndbg=+p apple_t2_vhci_hcd.dyndbg=+p snd_apple_t2.dyndbg=+p t2_ave.dyndbg=+p t2_sidecar.dyndbg=+p
```

## Support

If this work helps you and you want to support it:

https://www.paypal.com/paypalme/negmaster
