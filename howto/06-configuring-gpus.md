# How to configure GPUs

[Automatic installation](automatic-installation.md)

Manual installation:

1. [Introduction](00-introduction.md)
2. [Get Broadcom firmware from macOS](01-get-broadcom-firmware.md)
3. [Prepare macOS and the Fedora installer](02-prepare-macos-and-fedora-usb.md)
4. [Install Broadcom firmware on Fedora](03-install-broadcom-firmware.md)
5. [Install KaiT2en modules and apps](04-install-kait2en-modules-and-apps.md)
6. [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md)
7. [Configure GPUs](06-configuring-gpus.md) (you are here)
8. [How to update](07-updating.md)

Previous: [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md) | Next: [How to update](07-updating.md)

If a Mac has a dGPU, it will use it for boot and it will also use it as primary
display adapter by default. An iMac is no exception in that aspect, but it is
not able to switch between internal and dedicated GPU because the display lines
from iGPU to display are missing. So on iMacs, the iGPU is only used for offloading.
Thus, if you are an iMac user, this guide is not for you.
Same for Mac Pro users, since Mac Pros have no iGPU.
This guide is only for Macbook Pro users.

## Set the iGPU as primary display adapter

Instead of forcing the iGPU from GRUB, we simply do the equivalent using efivars.
Run this in a terminal:

```bash
# 1. remove the immutable-flag
sudo chattr -i /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9

# 2. write the iGPU value 
printf '\x07\x00\x00\x00\x00\x00\x00\x00' | sudo tee /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9

# 3. add the immutable flag agin so that macOS won't overwrite it
sudo chattr +i /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9
```

For making the dGPU primary again we only have to change one bit:

```bash
# 1. remove the immutable-flag
sudo chattr -i /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9

# 2. write the dGPU value 
printf '\x07\x00\x00\x00\x01\x00\x00\x00' | sudo tee /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9

# 3. add the immutable flag agin so that macOS won't overwrite it
sudo chattr +i /sys/firmware/efi/efivars/gpu-power-prefs-fa4ce28d-b62f-4c99-9cc3-6815686e30f9
```

After reboot, your Macbook will use the GPU of your choice as primary.

## Shutting down the dGPU

To save on power draw, we can electrically shut down the dGPU if we don't need it.
We will create a systemd service that will run on boot and do that for us using
vgaswitcheroo. On a Macbook 15,1 this will save us 12W on idle and literally split
power consumption in half.

Open a terminal and run:

```bash
sudo tee /etc/systemd/system/kait2en-dgpu-off.service >/dev/null <<'EOF'
[Unit]
Description=turns the dedicated gpu off on boot

[Service]
ExecStart=/bin/sh -c "echo OFF > /sys/kernel/debug/vgaswitcheroo/switch"

[Install]
WantedBy=multi-user.target

EOF

sudo systemctl daemon-reload
sudo systemctl enable kait2en-dgpu-off.service 
```

When rebooting, the dGPU will be turned off.
The trade-off for less power draw is, that we won't be able to use the dGPU
to accelerate games and apps. If you want to use it again, you will need to
disable the service and reboot:

```bash
sudo systemctl disable kait2en-dgpu-off.service 
sudo reboot
```

And of course the other way around when you want to enable the service again:

```bash
sudo systemctl enable kait2en-dgpu-off.service 
sudo reboot
```

You can make this process less awkward by adding aliases:

```bash
alias dgpu-off='sudo systemctl enable kait2en-dgpu-off.service; sleep 2; sudo reboot'
alias dgpu-on='sudo systemctl disable kait2en-dgpu-off.service; sleep 2; sudo reboot'
alias dgpu-status='sudo cat /sys/kernel/debug/vgaswitcheroo/switch'
```

From now on, when you enter `dgpu-off`, it will enable the service and reboot
while `dgpu-on` will disable it and reboot, and `dgpu-status` will show you if
the dGPU is currently on or off.

## MacBook Pro 15,1 A1990 dGPU suspend issues

On the MacBook 15,1 the SMU will die on suspend and resume with a black screen.
There are workarounds for that on driver level: like Fred's `amdgpu` ASIC patch
and also !ruicon is working on fixing gmux in the T2 Linux community.
But we didn't include that in KaiT2en modules because the patches are very WIP.

Anyways, if you want working suspend, you will need to configure iGPU as primary
and dGPU turned off as described above.
Also KaiT2en will automatically install a script that will `modprobe -r amdgpu`
on suspend when it finds a 15,1.

Previous: [Revert T2 Linux Fedora to vanilla + KaiT2en](05-revert-t2linux-fedora.md) | Next: [How to update](07-updating.md)
