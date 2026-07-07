# t2bce_core

Buffer Copy Engine fork for Intel Macs with a T2 chip with stateful and no-state resume support.

`t2bce_core` owns the T2 BCE PCI device, mailbox, PM coordination, and client
transport API. The shared DMA queue engine lives next to it in `modules/t2bce_dma`
and must be built/loaded before `t2bce_core`.

## Required kernel parameters

These kernel parameters have to be set in Linux commandline:

- `mem_sleep_default=deep` This is S3. S2 and 4 should also work
- `pm_async=off` needed on some machines to make pm ordering sequential. Since V0.04 it should work without it.
- `acpi_osi=!Darwin acpi_osi=Linux` for using the correct ACPI tables


# Workarounds

A fixed t2bce doesn't fix suspend. While it was broken for many years, developers of other T2 patches/drivers did not take care of implementing suspend/resume methods because suspend was broken by t2bce anyways. Also Apple never cared for Linux (on some Macbooks ACPI tables have real issues). And finally we have the T2 and Apple-modified hardware/platform to deal with. Not all hope is lost though. We can work around most of it using systemd units.
The below units will help you around the roughest cliffs. Note that you can combine them into one unit. Also note the minus sign in for example `ExecStart=-/usr/bin...` will let the systemd unit continue in case of error. For example if you haven't tiny-dfr installed, the service should still continue to execute - with cosmetic errors in journal. Feel free to remove what you don't need.
The code blocks are full commands. They will create the units and activate them. Copy them, modify them to your needs if you want and execute them. But don't forget the important bits like daemon-reload and systemctl enable.

## Notes for dGPU models

On dGPU Macs, suspend may still fail or resume may take very long unless the iGPU is set as the default GPU.
The 15,1 is notorious for a dead dGPU on resume (black screen with running fans). Do this:

```bash
echo "options t2gmux force_igd=y" | sudo tee /etc/modprobe.d/t2gmux.conf
```

Then create a systemd service to unload amdgpu when suspending by copy/pasting the whole code block below:
```
sudo tee /etc/systemd/system/amdgpu-suspend-fix.service >/dev/null <<'EOF'
[Unit]
Description=Unload and Reload Modules amdgpu for Suspend and Resume
Before=sleep.target
StopWhenUnneeded=yes

[Service]
User=root
Type=oneshot
RemainAfterExit=yes

ExecStart=-/usr/bin/rmmod -f amdgpu

ExecStop=-/usr/bin/modprobe amdgpu

[Install]
WantedBy=sleep.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable amdgpu-suspend-fix.service
```


## Notes for Macbooks with Touchbar

The touchbar/keyboard drivers are missing proper PM paths and the Touchbar will be broken after resuming. On some macs the kbd backlight stops working. On Macs with tiny-dfr installed, tiny DFR will sit on old FD's before suspend.

Run

`journalctl -b --grep="Product: Touch Bar Display"`
 
If needed, replace any occurencies of `3-6`  with the number from the output of the command above.

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

ExecStart=-/usr/bin/sh -c "/usr/bin/echo 0 | /usr/bin/tee /sys/class/leds/apple::kbd_backlight/brightness" # this is for butterfly keyboards 
ExecStart=-/usr/bin/sh -c "/usr/bin/echo 0 | /usr/bin/tee /sys/class/leds/:white:kbd_backlight/brightness" # this is for magic keyboards 
ExecStart=-/usr/bin/systemctl stop tiny-dfr.service
ExecStart=-/usr/bin/modprobe -r t2touchbar_kbd

ExecStop=-/usr/bin/modprobe t2touchbar_kbd
ExecStop=-/usr/bin/sh -c 'echo 0 > /sys/bus/usb/devices/3-6/bConfigurationValue'
ExecStop=-/usr/bin/sleep 1
ExecStop=-/usr/bin/sh -c 'echo 2 > /sys/bus/usb/devices/3-6/bConfigurationValue'
ExecStop=-/usr/bin/udevadm settle
ExecStop=-/usr/bin/systemctl restart tiny-dfr.service
ExecStopPost=-/usr/bin/sh -c "/usr/bin/echo 200 | /usr/bin/tee /sys/class/leds/apple::kbd_backlight/brightness"
ExecStopPost=-/usr/bin/sh -c "/usr/bin/echo 200 | /usr/bin/tee /sys/class/leds/:white:kbd_backlight/brightness"


[Install]
WantedBy=sleep.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable touchbar-suspend-fix.service
```

## Notes for Macbooks with BCM4377 Wifi

The 4377 chip is notorious for not managing to transition between power states. Thus it will refuse to enter sleep and wake the system again with all VHCI devices, like keyboard and trackpad not working. Do this:

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
ExecStop=-/usr/bin/modprobe hci_bcm4377


[Install]
WantedBy=sleep.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable 4377-suspend-fix.service
```


## Build and deploy

Download the zip file and go into the extracted t2bce_core folder with terminal. Type

```bash
make && sudo make install
sudo depmod -a
sudo reboot
```
After reboot type `modinfo t2bce_core`. The output should show a version number > `0.04` and `author: André Eikmeyer <andre.eikmeyer@gmail.com>`
 

## DKMS

Install the source tree and register the module with DKMS (t2bce_core version 0.06 as example here):

```bash
version=0.06
sudo install -d "/usr/src/t2bce_core-${version}"
git archive --format=tar HEAD | sudo tar -x -C "/usr/src/t2bce_core-${version}"
sudo dkms add -m t2bce_core -v "${version}"
sudo dkms install -m t2bce_core -v "${version}"
```

DKMS will automatically rebuild the module for newly installed kernels.

To remove the DKMS installation:

```bash
sudo dkms remove -m t2bce_core -v 0.06 --all
sudo rm -rf /usr/src/t2bce_core-0.06
```

## Support

If this work helps you and you want to support it:

https://www.paypal.com/paypalme/negmaster
