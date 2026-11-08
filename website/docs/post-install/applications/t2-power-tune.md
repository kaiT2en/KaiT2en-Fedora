# T2 Power Tune

<img class="app-shot" src="img/apps/t2-power-tune.webp" alt="T2 Power Tune with package C-state residency, PCIe ASPM and LTR options">

Lists PCIe ASPM link states, LTR requirements, runtime power management, wake
sources and other power tunables, and shows the package C-state residency of
the last second. It replaces powertop and TLP for reaching deeper package
C-states.

- **Apply selected changes** writes the selection for the current boot and
  verifies it. Changing a checkbox writes nothing.
- **Create systemd service** stores the selection in
  `kait2en-power-tune.service`, which applies it at every boot.
- **Rescan** reads the hardware state again.

Change one item at a time, check the C-state residency and test suspend and
resume before making it persistent. More enabled options do not mean more
savings. Typical failures are missing Wi-Fi or Bluetooth after suspend,
PCIe errors, audio dropouts and I/O timeouts. Leave BCM4377 runtime PM off
unless suspend and resume were tested. The help button of each section
explains its risks.

## Reset

```bash
sudo systemctl disable --now kait2en-power-tune.service
sudo rm /etc/systemd/system/kait2en-power-tune.service
sudo systemctl daemon-reload
```

`--now` restores the values saved before the service applied its changes.
If the system no longer boots, add the kernel argument
`systemd.mask=kait2en-power-tune.service` once in GRUB and run the commands
above.
