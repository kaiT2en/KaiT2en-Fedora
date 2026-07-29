# KaiT2en Fedora installer USB

This kit prepares one Fedora USB stick on macOS. It adds the T2 keyboard and
trackpad drivers, brings up Wi-Fi in the live system, and carries the Apple
Wi-Fi firmware into the installed system. On Macs whose Bluetooth runs over
PCIe (BCM4377) it does the same for the Apple Bluetooth firmware.

For a published release, download, verify, and start the complete kit with:

```bash
curl -fsSL https://github.com/kaiT2en/KaiT2en-Fedora/releases/latest/download/install-kait2en-fedora.sh | bash
```

The repository checkout method remains available below for development builds.

1. Connect an empty USB drive and run:

   ```bash
   bash ./scripts/macos/prepare-fedora-installer.sh
   ```

2. Choose Fedora Workstation, KDE, or COSMIC. Then
   choose the external USB drive. The script downloads and verifies Fedora
   before asking for the final `ERASE diskN` confirmation.
3. Boot the T2 Mac from that USB drive and install Fedora normally. Format an
   existing Linux `/boot` partition when doing a clean reinstall. Wi-Fi is
   available in the live system itself, which also makes the stick usable for
   rescue work. Run `sudo /run/kait2en/kait2en-live-wifi` if no wireless network
   shows up there, `sudo /run/kait2en/kait2en-live-bluetooth` if a BCM4377 Mac
   finds no Bluetooth controller, and
   `sudo /run/kait2en/kait2en-live-diagnostics --rerun` to write one diagnostics
   archive to a second USB drive.
4. Connect to Wi-Fi after the first login. One guided KaiT2en installer opens
   automatically and updates Fedora. Reboot when its first phase succeeds.
   If no terminal appears, open one and run `kait2en-install` without
   `sudo`; the installer requests administrator access when needed.
5. After the reboot, the same installer continues automatically with the
   regular, unchanged KaiT2en installer from GitHub `main`. Review its output
   and reboot after success.

Run `kait2en-install` again at any later time to fast-forward the clean Git
checkout and rerun the regular project installer.

The standard Fedora boot entry remains available on the USB stick as a fallback.
Atomic desktops, Labs, Server, and network-install images are not supported by
this installer kit.

Bluetooth firmware is only installed on Macs with the BCM4377 PCIe controller
(`14e4:5fa0`); every other T2 Mac uses the UART controller and needs none. The
kernel driver recognises a limited set of models, currently MacBookAir9,1,
MacBookPro15,4 and MacBookPro16,3. On any other model it reports
`unable to determine board type` and stops before it asks for firmware, which
the installer and the diagnostics archive both point out. Please report such a
model instead of installing firmware by hand.

For an already downloaded ISO or scripted use, see:

```bash
bash ./scripts/macos/prepare-fedora-installer.sh --help
```

## Maintainers

Build a target from a clean Git checkout:

```bash
./scripts/fedora/build-installer.sh --target fedora-44
```

A Fedora release consists of one `fedora-N.conf` target, one
`fedora-N-editions.tsv` catalog, and the target-specific input compatibility
patch. CI checks every catalog entry against Fedora's official `releases.json`.
