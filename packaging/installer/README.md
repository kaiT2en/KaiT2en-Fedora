# KaiT2en Fedora installer USB

This kit prepares one Fedora USB stick on macOS. It adds the T2 keyboard and
trackpad drivers and carries the Apple Wi-Fi firmware into the installed system.

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
   existing Linux `/boot` partition when doing a clean reinstall.
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
