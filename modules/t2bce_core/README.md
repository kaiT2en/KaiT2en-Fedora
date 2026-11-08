# t2bce_core

Buffer Copy Engine driver for T2 Macs, with resume from both stateful and
stateless bridgeOS.

`t2bce_core` owns the BCE PCI device, the mailbox, PM coordination and the
client transport API. The DMA queue engine lives in `modules/t2bce_dma` and
loads first.

## Kernel parameters

- `mem_sleep_default=deep` (S3). s2idle and hibernation should work too.

## Workarounds

Other drivers still lack parts of their suspend handling. The systemd units
below cover that. Each block creates and enables its unit. The `-` in
`ExecStart=-` keeps a unit going when a step fails, for example without
tiny-dfr.

## Notes for MacBookPro15,1, MacBookPro16,1 and MacBookPro16,4 graphics

KaiT2en carries the apple-gmux, AMDGPU and HDA patches for hybrid graphics on
these models. Runtime dGPU power management and suspend work on all three.

## Notes for Macbooks with Touchbar

Without this unit the Touch Bar, and on some Macs the keyboard backlight,
stay dead after resume, and tiny-dfr holds stale file descriptors. Replace
`3-6` with the USB path from:

```
journalctl -b --grep="Product: Touch Bar Display"
```

```
sudo tee /etc/systemd/system/touchbar-suspend-fix.service >/dev/null <<'EOF'
[Unit]
Description=Unload and Reload Modules for Suspend and Resume
Before=sleep.target
StopWhenUnneeded=yes

[Service]
User=root
Type=oneshot
RemainAfterExit=yes

ExecStart=-/usr/bin/sh -c "/usr/bin/echo 0 | /usr/bin/tee /sys/class/leds/:white:kbd_backlight/brightness"
ExecStart=-/usr/bin/systemctl stop tiny-dfr.service
ExecStart=-/usr/bin/modprobe -r t2touchbar_kbd

ExecStop=-/usr/bin/modprobe t2touchbar_kbd
ExecStop=-/usr/bin/sh -c 'echo 0 > /sys/bus/usb/devices/3-6/bConfigurationValue'
ExecStop=-/usr/bin/sleep 1
ExecStop=-/usr/bin/sh -c 'echo 2 > /sys/bus/usb/devices/3-6/bConfigurationValue'
ExecStop=-/usr/bin/udevadm settle
ExecStop=-/usr/bin/systemctl restart tiny-dfr.service
ExecStopPost=-/usr/bin/sh -c "/usr/bin/echo 200 | /usr/bin/tee /sys/class/leds/:white:kbd_backlight/brightness"


[Install]
WantedBy=sleep.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable touchbar-suspend-fix.service
```

## Notes for Macbooks with BCM4377 Wifi

The BCM4377 fails its power state transitions, so the system refuses to sleep
or wakes with keyboard and trackpad dead:

```
sudo tee /etc/systemd/system/4377-suspend-fix.service >/dev/null <<'EOF'
[Unit]
Description=Unload and Reload BCM4377 for Suspend and Resume
Before=sleep.target
StopWhenUnneeded=yes

[Service]
User=root
Type=oneshot
RemainAfterExit=yes

ExecStart=-/usr/bin/rmmod hci_bcm4377
ExecStart=-/usr/bin/rmmod brcmfmac_wcc
ExecStart=-/usr/bin/rmmod brcmfmac

ExecStop=-/usr/bin/modprobe brcmfmac
ExecStop=-/usr/bin/modprobe brcmfmac_wcc
ExecStop=-/usr/bin/sleep 5
ExecStop=-/usr/bin/modprobe hci_bcm4377


[Install]
WantedBy=sleep.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable 4377-suspend-fix.service
```


## Build and deploy

```bash
make && sudo make install
sudo depmod -a
sudo reboot
```

## DKMS

The Fedora installer registers `t2bce_core` together with `t2bce_dma`,
`t2bce_vhci`, `t2bce_audio`, and `t2bce_ave` as the single `t2bce_stack` DKMS package. This
keeps their exported symbols and ABI synchronized. Do not register this source
directory as a separate DKMS package.

## Dev Debug

Most messages are `pr_debug`. For full output:

```
t2bce_vhci.dyndbg=+p t2bce_dma.dyndbg=+p t2bce_core.dyndbg=+p t2bce_audio.dyndbg=+p t2bce_ave.dyndbg=+p
```

## Support

If this work helps you and you want to support it:

https://www.paypal.com/paypalme/negmaster
