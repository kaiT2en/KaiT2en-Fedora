# T2 Power Tune

## Reset

```sh
sudo systemctl disable --now kait2en-power-tune-resume.service kait2en-power-tune.service
sudo rm -f /etc/systemd/system/kait2en-power-tune-resume.service
sudo rm /etc/systemd/system/kait2en-power-tune.service
sudo rm -f /run/t2-power-tune/items.json
sudo systemctl daemon-reload
```

`--now` runs the unit's `ExecStop` commands and restores the saved values.
`kait2en-power-tune-resume.service` applies the same selection again after
every resume.

## Recovery when the system does not boot

Add this kernel parameter once in the bootloader:

```text
systemd.mask=kait2en-power-tune.service systemd.mask=kait2en-power-tune-resume.service
```

After booting:

```sh
sudo systemctl disable kait2en-power-tune-resume.service kait2en-power-tune.service
sudo rm -f /etc/systemd/system/kait2en-power-tune-resume.service
sudo rm /etc/systemd/system/kait2en-power-tune.service
sudo systemctl daemon-reload
sudo reboot
```

`/run/t2-power-tune/items.json` is volatile and disappears on reboot.
